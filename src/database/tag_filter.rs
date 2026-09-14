//! Comma-separated tag predicates. Every predicate must match (AND).
pub fn terms(filter: &str) -> impl Iterator<Item = (bool, &str)> {
    filter.split(',').filter_map(|term| {
        let term = term.trim();
        let (excluded, slug) = term
            .strip_prefix('!')
            .map_or((false, term), |s| (true, s.trim()));
        (!slug.is_empty()).then_some((excluded, slug))
    })
}

pub fn selected(filter: &str, slug: &str, excluded: bool) -> bool {
    terms(filter).any(|term| term == (excluded, slug))
}

pub fn matches(filter: &str, mut has_tag: impl FnMut(&str) -> bool) -> bool {
    terms(filter).all(|(excluded, slug)| has_tag(slug) != excluded)
}

pub fn json(filter: &str) -> String {
    serde_json::to_string(&terms(filter).collect::<Vec<_>>())
        .expect("tag predicates are serializable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicates_are_conjoined_and_normalized() {
        let has = |slug: &str| ["keep", "review"].contains(&slug);
        assert!(matches("keep,review,!remove", has));
        assert!(!matches("keep,!review", has));
        assert!(!matches("keep,missing", has));
        assert!(matches(" , ! , keep, keep ", has));
        assert!(matches("", has));
        assert!(selected("keep,!review", "review", true));
        assert!(!selected("keeper", "keep", false));
    }
}
