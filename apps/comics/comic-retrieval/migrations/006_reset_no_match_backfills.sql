-- One-time fix: every 'no_match' backfill so far was produced by
-- best_bulk_match, which only ever matched a single torrent covering an
-- entire volume range. nyaa releases manga one torrent per volume, not as
-- combined ranges, so best_bulk_match structurally could never match and
-- every backfill was marked 'no_match' regardless of what nyaa actually had.
-- Reset them to 'needed' so the corrected per-volume matcher (match_volumes)
-- gets a real chance at them.
UPDATE backfills SET status = 'needed', updated_at = updated_at WHERE status = 'no_match';
