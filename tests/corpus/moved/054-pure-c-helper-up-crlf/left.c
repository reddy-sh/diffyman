#include <stdlib.h>
#include "list.h"

int list_push(struct list *list, int value)
{
  struct node *n = node_new(value);
  if (n == NULL) {
    return -1;
  }
  n->next = list->head;
  list->head = n;
  list->length++;
  return 0;
}

static struct node *node_new(int value)
{
  struct node *n = malloc(sizeof *n);
  if (n != NULL) {
    n->value = value;
    n->next = NULL;
  }
  return n;
}
