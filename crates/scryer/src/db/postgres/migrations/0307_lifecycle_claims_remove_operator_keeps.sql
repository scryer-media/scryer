-- Keeping a title is a tag rule's job, and keeping a leased title permanently
-- only ends its lease. Operator keeps, live and historical, are removed. A
-- lease converted to permanent stays converted, so its title carries no
-- expiry.
DELETE FROM lifecycle_claims
WHERE producer = 'operator_keep';
