#include <stdio.h>

static int add(int a, int b)
{
    return a + b;
}

static int sub(int a, int b)
{
    return a - b;
}

int main(void)
{
    printf("%d\n", add(2, 3));
    return 0;
}
