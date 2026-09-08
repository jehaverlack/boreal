CREATE TABLE metadata_timing_history (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    duration_seconds INTEGER NOT NULL CHECK(duration_seconds > 0),
    completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX metadata_timing_history_source_completed_idx
    ON metadata_timing_history(source, completed_at DESC);

-- Preserve useful estimates from earlier releases. New runs record isolated
-- per-source durations and will naturally replace these among the latest five.
INSERT INTO metadata_timing_history(source, duration_seconds, completed_at)
SELECT scan_type,
       MAX(1, CAST(ROUND((julianday(completed_at) - julianday(started_at)) * 86400) AS INTEGER)),
       completed_at
FROM scan_runs
WHERE scan_type IN ('my-drive', 'shared-with-me', 'shared-drives')
  AND status = 'complete' AND completed_at IS NOT NULL
ORDER BY id ASC;
