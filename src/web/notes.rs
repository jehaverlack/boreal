use crate::app::AppState;
use axum::{Json, extract::State, http::StatusCode};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Target {
    kind: String,
    key: String,
}
impl Target {
    fn valid(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "drive" | "keeper-folder" | "keeper-record" | "github" | "local"
        ) && !self.key.trim().is_empty()
            && self.key.len() <= 8192
    }
}
#[derive(Serialize)]
pub struct Note {
    id: i64,
    body: String,
    html: String,
    revision: i64,
}
#[derive(Serialize)]
pub struct Notes {
    target: Target,
    notes: Vec<Note>,
}
#[derive(Deserialize)]
pub struct Save {
    target: Target,
    id: Option<i64>,
    revision: Option<i64>,
    body: String,
}
#[derive(Deserialize)]
pub struct Delete {
    target: Target,
    id: i64,
    revision: i64,
}
type Failure = (StatusCode, String);
fn failure(error: impl std::fmt::Display) -> Failure {
    log::error!("Notes: {error}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unable to access notes. Please retry.".into(),
    )
}

fn render(body: &str) -> String {
    use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd, html};
    let mut depth = 0;
    let mut events = Vec::new();
    let finder = linkify::LinkFinder::new();
    for event in Parser::new_ext(
        body,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    ) {
        match event {
            Event::Start(ref tag)
                if matches!(
                    tag,
                    Tag::Link { .. } | Tag::Image { .. } | Tag::CodeBlock(_)
                ) =>
            {
                depth += 1;
                events.push(event);
            }
            Event::End(tag) if matches!(tag, TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock) => {
                depth -= 1;
                events.push(Event::End(tag));
            }
            Event::Html(text) | Event::InlineHtml(text) => events.push(Event::Text(text)),
            Event::Text(text) if depth == 0 => {
                let mut end = 0;
                for link in finder.links(&text) {
                    events.push(Event::Text(text[end..link.start()].to_owned().into()));
                    let url = if link.kind() == &linkify::LinkKind::Email {
                        format!("mailto:{}", link.as_str())
                    } else {
                        link.as_str().to_owned()
                    };
                    events.push(Event::Start(Tag::Link {
                        link_type: LinkType::Inline,
                        dest_url: url.into(),
                        title: "".into(),
                        id: "".into(),
                    }));
                    events.push(Event::Text(link.as_str().to_owned().into()));
                    events.push(Event::End(TagEnd::Link));
                    end = link.end();
                }
                events.push(Event::Text(text[end..].to_owned().into()));
            }
            _ => events.push(event),
        }
    }
    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());
    ammonia::Builder::default()
        .url_schemes(["https", "http", "mailto"].into_iter().collect())
        .url_relative(ammonia::UrlRelative::Deny)
        .set_tag_attribute_value("a", "target", "_blank")
        .link_rel(Some("noopener noreferrer"))
        .clean(&output)
        .to_string()
}
fn read(connection: &rusqlite::Connection, target: Target) -> Result<Notes, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT id, body, revision FROM item_notes WHERE kind=?1 AND item_key=?2 ORDER BY id",
    )?;
    let notes = statement
        .query_map(params![target.kind, target.key], |row| {
            let body: String = row.get(1)?;
            Ok(Note {
                id: row.get(0)?,
                html: render(&body),
                body,
                revision: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Notes { target, notes })
}
pub async fn list(
    State(state): State<Arc<AppState>>,
    Json(targets): Json<Vec<Target>>,
) -> Result<Json<Vec<Notes>>, Failure> {
    if targets.len() > 200 || targets.iter().any(|target| !target.valid()) {
        return Err((StatusCode::BAD_REQUEST, "Invalid note targets.".into()));
    }
    let connection = state
        .database()
        .map_err(failure)?
        .connect()
        .map_err(failure)?;
    Ok(Json(
        targets
            .into_iter()
            .map(|target| read(&connection, target))
            .collect::<Result<_, _>>()
            .map_err(failure)?,
    ))
}
pub async fn save(
    State(state): State<Arc<AppState>>,
    Json(input): Json<Save>,
) -> Result<Json<Notes>, Failure> {
    let connection = state
        .database()
        .map_err(failure)?
        .connect()
        .map_err(failure)?;
    store(&connection, input).map(Json)
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    Json(input): Json<Delete>,
) -> Result<Json<Notes>, Failure> {
    let connection = state
        .database()
        .map_err(failure)?
        .connect()
        .map_err(failure)?;
    remove(&connection, input).map(Json)
}

fn remove(connection: &rusqlite::Connection, input: Delete) -> Result<Notes, Failure> {
    if !input.target.valid() || input.id <= 0 || input.revision <= 0 {
        return Err((StatusCode::BAD_REQUEST, "Invalid note selection.".into()));
    }
    let changed = connection
        .execute(
            "DELETE FROM item_notes WHERE id=?1 AND kind=?2 AND item_key=?3 AND revision=?4",
            params![
                input.id,
                input.target.kind,
                input.target.key,
                input.revision
            ],
        )
        .map_err(failure)?;
    if changed == 0 {
        return Err((
            StatusCode::CONFLICT,
            "This note changed or was deleted in another window. Reload before trying again."
                .into(),
        ));
    }
    read(connection, input.target).map_err(failure)
}

fn store(connection: &rusqlite::Connection, input: Save) -> Result<Notes, Failure> {
    if !input.target.valid() || input.body.trim().is_empty() || input.body.len() > 65536 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Enter a note between 1 and 65,536 bytes.".into(),
        ));
    }
    if let Some(id) = input.id {
        let changed = connection.execute("UPDATE item_notes SET body=?1, revision=revision+1, updated_at=CURRENT_TIMESTAMP WHERE id=?2 AND kind=?3 AND item_key=?4 AND revision=?5",
            params![input.body, id, input.target.kind, input.target.key, input.revision]).map_err(failure)?;
        if changed == 0 {
            return Err((
                StatusCode::CONFLICT,
                "This note changed in another window. Copy your edits, reload, and try again."
                    .into(),
            ));
        }
    } else {
        connection
            .execute(
                "INSERT INTO item_notes(kind,item_key,body) VALUES (?1,?2,?3)",
                params![input.target.kind, input.target.key, input.body],
            )
            .map_err(failure)?;
    }
    read(connection, input.target).map_err(failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_links_and_untrusted_html() {
        let html = render(
            "**bold** [site](https://example.org) https://example.com\n\n<script>alert(1)</script>\n\n[bad](javascript:alert%281%29)",
        );
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("href=\"https://example.org\""));
        assert!(html.contains("href=\"https://example.com\""));
        assert!(html.contains("target=\"_blank\""));
        assert!(html.contains("noopener noreferrer"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("href=\"javascript:"));
        assert!(!render("`https://example.org`").contains("<a"));
    }
    #[test]
    fn multiple_notes_are_isolated_by_item() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(include_str!("../database/migrations/0039_item_notes.sql"))
            .unwrap();
        connection.execute_batch("INSERT INTO item_notes(kind,item_key,body) VALUES ('drive','a','one'),('drive','a','two'),('keeper-record','a','other');").unwrap();
        let result = read(
            &connection,
            Target {
                kind: "drive".into(),
                key: "a".into(),
            },
        )
        .unwrap();
        assert_eq!(result.notes.len(), 2);
        assert_eq!(result.notes[1].body, "two");
        assert_eq!(
            read(
                &connection,
                Target {
                    kind: "drive".into(),
                    key: "b".into()
                }
            )
            .unwrap()
            .notes
            .len(),
            0
        );
    }
    #[test]
    fn edits_validate_target_revision_and_body() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(include_str!("../database/migrations/0039_item_notes.sql"))
            .unwrap();
        let input = |key: &str, id, revision, body: &str| Save {
            target: Target {
                kind: "drive".into(),
                key: key.into(),
            },
            id,
            revision,
            body: body.into(),
        };
        let created = store(&connection, input("a", None, None, "First")).unwrap();
        let id = created.notes[0].id;
        store(&connection, input("a", None, None, "Second")).unwrap();
        let edited = store(&connection, input("a", Some(id), Some(1), "Edited")).unwrap();
        assert_eq!(edited.notes.len(), 2);
        assert_eq!(edited.notes[0].body, "Edited");
        assert_eq!(edited.notes[0].revision, 2);
        assert!(matches!(
            store(&connection, input("a", Some(id), Some(1), "Stale")),
            Err((StatusCode::CONFLICT, _))
        ));
        assert!(matches!(
            store(&connection, input("b", Some(id), Some(2), "Wrong item")),
            Err((StatusCode::CONFLICT, _))
        ));
        assert!(matches!(
            store(&connection, input("a", None, None, "  ")),
            Err((StatusCode::BAD_REQUEST, _))
        ));
        assert!(matches!(
            store(&connection, input("a", None, None, &"x".repeat(65537))),
            Err((StatusCode::BAD_REQUEST, _))
        ));
    }
    #[test]
    fn deletion_checks_target_and_revision_and_preserves_other_notes() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(include_str!("../database/migrations/0039_item_notes.sql"))
            .unwrap();
        connection.execute_batch("INSERT INTO item_notes(kind,item_key,body) VALUES ('drive','a','one'),('drive','a','two'),('keeper-record','a','other');").unwrap();
        let input = |kind: &str, key: &str, revision| Delete {
            target: Target {
                kind: kind.into(),
                key: key.into(),
            },
            id: 1,
            revision,
        };
        for invalid in [
            input("drive", "b", 1),
            input("keeper-record", "a", 1),
            input("drive", "a", 2),
        ] {
            assert!(matches!(
                remove(&connection, invalid),
                Err((StatusCode::CONFLICT, _))
            ));
        }
        let remaining = remove(&connection, input("drive", "a", 1)).unwrap();
        assert_eq!(remaining.notes.len(), 1);
        assert_eq!(remaining.notes[0].body, "two");
        assert!(matches!(
            remove(&connection, input("drive", "a", 1)),
            Err((StatusCode::CONFLICT, _))
        ));
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM item_notes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }
}
