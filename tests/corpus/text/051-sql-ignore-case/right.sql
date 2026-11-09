SELECT id, name FROM customers
WHERE city = 'ÉTÉ' AND active = 1
ORDER BY name;

UPDATE streets SET name = 'STRASSE' WHERE id = 7;
DELETE FROM sessions WHERE expires < NOW();
