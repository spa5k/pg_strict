-- pg_strict regression tests

CREATE TABLE pg_strict_test(id int primary key, flag boolean);
INSERT INTO pg_strict_test VALUES (1, true), (2, false);

-- Ensure default is off
SHOW pg_strict.require_where_on_update;
SHOW pg_strict.require_where_on_delete;

-- Turning on update enforcement should block UPDATE without WHERE
SET pg_strict.require_where_on_update = 'on';
UPDATE pg_strict_test SET flag = false;

-- But allow a safe UPDATE
UPDATE pg_strict_test SET flag = true WHERE id = 1;

-- Turning on delete enforcement should block DELETE without WHERE
SET pg_strict.require_where_on_delete = 'on';
DELETE FROM pg_strict_test;

-- But allow a safe DELETE
DELETE FROM pg_strict_test WHERE id = 2;

-- Warn mode should not block execution
SET pg_strict.require_where_on_update = 'warn';
UPDATE pg_strict_test SET flag = false;

-- Reset to default
RESET pg_strict.require_where_on_update;
RESET pg_strict.require_where_on_delete;

DROP TABLE pg_strict_test;
