public sealed class Thermostat
{
    private const double Step = 0.5;
    private double target = 20.0;

    public void Raise() => target += Step;
    public void Lower() => target -= Step;

    public double Target => target;
}
