#include <stdio.h>

static int add(int a, int b)
{
    return a + b;
}

static int mul(int a, int b)
{
    return a * b;
}

static int sub(int a, int b)
{
    return a - b;
}

int main(void)
{
    printf("%d\n", mul(add(2, 3), sub(7, 4)));
    return 0;
}
