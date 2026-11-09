public sealed class Thermostat
{
    private double target = 20.0;

    public void Raise() { target = target + 0.5; }
    public void Lower() { target = target - 0.5; }

    public double Target => target;
}
