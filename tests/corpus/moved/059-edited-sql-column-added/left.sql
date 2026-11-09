-- monthly customer report

SELECT c.name, c.email,
  SUM(o.total) AS spent
FROM clients c JOIN orders o ON o.client_id = c.id
WHERE o.placed_at >= :since
GROUP BY c.name, c.email;

SELECT status, COUNT(*) AS orders
FROM orders
GROUP BY status
ORDER BY orders DESC;
