CREATE TABLE customers (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  email TEXT UNIQUE
);

CREATE VIEW open_orders AS
SELECT o.id, o.placed_at, c.name
FROM orders o
JOIN customers c ON c.id = o.customer_id
WHERE o.status = 'open';

CREATE TABLE orders (
  id INTEGER PRIMARY KEY,
  customer_id INTEGER NOT NULL REFERENCES customers (id),
  placed_at TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'open'
);
