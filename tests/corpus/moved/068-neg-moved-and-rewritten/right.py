import math

def midpoint(a, b):
  return ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)

def distance(p, q):
  return math.hypot(q.x - p.x, q.y - p.y)
