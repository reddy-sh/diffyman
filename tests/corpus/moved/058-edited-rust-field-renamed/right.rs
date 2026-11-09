pub struct Ring<T> {
  items: Vec<T>,
  capacity: usize,
}

pub const DEFAULT_CAPACITY: usize = 16;

fn fill(ring: &mut Ring<u8>) -> usize {
  let mut added = 0;
  while !ring.is_full() {
    ring.items.push(0);
    added += 1;
  }
  added
}

impl<T> Ring<T> {
  pub fn new(capacity: usize) -> Self {
    Ring { items: Vec::with_capacity(capacity), capacity }
  }
  pub fn is_full(&self) -> bool {
    self.items.len() >= self.capacity
  }
}
