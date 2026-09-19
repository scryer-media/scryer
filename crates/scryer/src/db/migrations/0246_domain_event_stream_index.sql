-- Index the domain-event log by stream.
--
-- A stream reader wants one stream: the library-scan waiter wants the events
-- of the scan session it is waiting on, and nothing else. `domain_events` had
-- no index whose leading column is `stream_id`, so the only way to read a
-- stream was to seek every row of the relevant `event_type` values and throw
-- the other streams away in memory. On a 12k-title scan that meant fetching
-- every stored library-scan event (~73k rows, the whole table's worth of
-- pages) to answer a question about one session.
--
-- `(stream_id, sequence)` matches both halves of how a stream is read --
-- equality on the stream, then a `sequence > cursor` range in sequence order
-- -- so the read seeks straight to the cursor and stops at the limit, with no
-- sorter. The index is partial because a global event carries no stream id and
-- can never satisfy `stream_id = ?`; leaving those rows out keeps the index
-- off the hot append path for every event that has no stream.

CREATE INDEX IF NOT EXISTS idx_domain_events_stream_sequence
    ON domain_events (stream_id, sequence)
    WHERE stream_id IS NOT NULL;
