import logging

log = logging.getLogger(__name__)


def is_valid(order, inventory):
    if order.quantity <= 0:
        log.warning("order %s has no quantity", order.id)
        return False
    if order.sku not in inventory:
        log.warning("order %s has an unknown sku", order.id)
        return False
    return True


def process_orders(orders, inventory):
    accepted = []
    for order in orders:
        if not is_valid(order, inventory):
            continue
        inventory[order.sku] -= order.quantity
        accepted.append(order)
    return accepted
