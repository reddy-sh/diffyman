export class Cart {
  private items: Item[] = [];

  add(item: Item): void {
    this.items.push(item);
  }

  remove(sku: string): void {
    this.items = this.items.filter((item) => item.sku !== sku);
  }

  total(): number {
    let sum = 0;
    for (const item of this.items) {
      sum += item.price * item.quantity;
    }
    return sum;
  }
}
