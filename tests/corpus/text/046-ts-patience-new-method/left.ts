export class Cart {
  private items: Item[] = [];

  add(item: Item): void {
    this.items.push(item);
  }

  total(): number {
    let sum = 0;
    for (const item of this.items) {
      sum += item.price * item.quantity;
    }
    return sum;
  }
}
