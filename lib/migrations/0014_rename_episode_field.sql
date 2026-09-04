-- Renames the stored Episode-kind field to match its preset.
--
-- The Episode kind's pattern names its capture group after the preset that
-- carries it, and `compose` wraps a component whose pattern names no group
-- under the field's own name. A stored field of that kind named `episode`
-- would therefore match the wrap rather than the inner group, and read `E06`
-- where it used to read `06`.
--
-- A test value follows its field. The join reaches the template a ruleset is
-- based on, because the field lives on the template while the test lives on
-- the ruleset. A value on a Number-kind field named `episode`, which carries
-- a pattern of its own, is left alone.
UPDATE ruleset_fields
SET name = 'episodeNumber'
WHERE name = 'episode' AND kind = 'episode';

UPDATE ruleset_test_values
SET field = 'episodeNumber'
WHERE field = 'episode'
  AND ruleset IN (
      SELECT r.id
      FROM rulesets r
      JOIN ruleset_fields f ON f.ruleset = r.id OR f.ruleset = r.based_on
      WHERE f.name = 'episodeNumber' AND f.kind = 'episode'
  );
