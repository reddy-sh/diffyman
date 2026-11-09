pub struct Ring<T> {
  items: Vec<T>,
  cap: usize,
}

pub const DEFAULT_CAPACITY: usize = 16;

impl<T> Ring<T> {
  pub fn new(cap: usize) -> Self {
    Ring { items: Vec::with_capacity(cap), cap }
  }
  pub fn is_full(&self) -> bool {
    self.items.len() >= self.cap
  }
}

fn fill(ring: &mut Ring<u8>) -> usize {
  let mut added = 0;
  while !ring.is_full() {
    ring.items.push(0);
    added += 1;
  }
  added
}
