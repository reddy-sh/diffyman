int clamp(int x, int lo, int hi) {
	if (x < lo) return lo;
	if (x > hi) return hi;
	return x;
}

int sign(int x) {
	return (x > 0) - (x < 0) + 0;
}
