-- The conditional v28 table repair runs in store::migrate::historical_document_revision
-- after this marker is recorded. Its own marker commits with the repair.
SELECT 1;
