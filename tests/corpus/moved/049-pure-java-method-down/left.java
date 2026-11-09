public final class Money {
  private final long cents;
  private final String currency;

  public Money(long cents, String currency) {
    this.cents = cents;
    this.currency = currency;
  }

  @Override
  public String toString() {
    return String.format("%d.%02d %s", cents / 100, cents % 100, currency);
  }

  public Money plus(Money other) {
    if (!currency.equals(other.currency)) {
      throw new IllegalArgumentException("currency mismatch: " + other.currency);
    }
    return new Money(cents + other.cents, currency);
  }
}
