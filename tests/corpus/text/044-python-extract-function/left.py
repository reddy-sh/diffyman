import logging

log = logging.getLogger(__name__)


def process_orders(orders, inventory):
    accepted = []
    for order in orders:
        if order.quantity <= 0:
            log.warning("order %s has no quantity", order.id)
            continue
        if order.sku not in inventory:
            log.warning("order %s has an unknown sku", order.id)
            continue
        inventory[order.sku] -= order.quantity
        accepted.append(order)
    return accepted
