-- monthly customer report

SELECT status, COUNT(*) AS orders
FROM orders
GROUP BY status
ORDER BY orders DESC;

SELECT c.name, c.email,
  SUM(o.total) AS spent,
  COUNT(*) AS paid_orders
FROM clients c JOIN orders o ON o.client_id = c.id
WHERE o.placed_at >= :start
GROUP BY c.name, c.email;
