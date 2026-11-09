export namespace Units {
  export const freezingFahrenheit = 32;

  export function toCelsius(fahrenheit: number): number {
    const delta = fahrenheit - 32;
    return (delta * 5) / 9;
  }
}
