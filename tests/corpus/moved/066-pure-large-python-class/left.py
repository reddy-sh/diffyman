import sys


class Record:
  def __init__(self):
    self._dirty = set()

  def set_owner(self, value):
    if not isinstance(value, int):
      raise TypeError("set_owner expects a number")
    self.owner = value
    self._dirty.add("owner")
    return self

  def set_title(self, value):
    if not isinstance(value, str):
      raise TypeError("set_title expects a string")
    self.title = value
    self._dirty.add("title")
    return self

  def set_price(self, value):
    if not isinstance(value, float):
      raise TypeError("set_price expects a decimal")
    self.price = value
    self._dirty.add("price")
    return self

  def set_stock(self, value):
    if not isinstance(value, int):
      raise TypeError("set_stock expects a number")
    self.stock = value
    self._dirty.add("stock")
    return self

  def set_label(self, value):
    if not isinstance(value, str):
      raise TypeError("set_label expects a string")
    self.label = value
    self._dirty.add("label")
    return self

  def set_color(self, value):
    if not isinstance(value, float):
      raise TypeError("set_color expects a decimal")
    self.color = value
    self._dirty.add("color")
    return self

  def set_width(self, value):
    if not isinstance(value, int):
      raise TypeError("set_width expects a number")
    self.width = value
    self._dirty.add("width")
    return self

  def set_depth(self, value):
    if not isinstance(value, str):
      raise TypeError("set_depth expects a string")
    self.depth = value
    self._dirty.add("depth")
    return self

  def changed(self):
    return sorted(self._dirty)


def report_00(records, out):
  selected = [r for r in records if r.stock > 0]
  out.write("report 00: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 0


def report_01(records, out):
  selected = [r for r in records if r.stock > 5]
  out.write("report 01: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 1


def report_02(records, out):
  selected = [r for r in records if r.stock > 10]
  out.write("report 02: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 2


def report_03(records, out):
  selected = [r for r in records if r.stock > 15]
  out.write("report 03: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 3


def report_04(records, out):
  selected = [r for r in records if r.stock > 20]
  out.write("report 04: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 4


def report_05(records, out):
  selected = [r for r in records if r.stock > 25]
  out.write("report 05: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 5


def report_06(records, out):
  selected = [r for r in records if r.stock > 30]
  out.write("report 06: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 6


def report_07(records, out):
  selected = [r for r in records if r.stock > 35]
  out.write("report 07: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 7


def report_08(records, out):
  selected = [r for r in records if r.stock > 40]
  out.write("report 08: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 8


def report_09(records, out):
  selected = [r for r in records if r.stock > 45]
  out.write("report 09: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 9


def report_10(records, out):
  selected = [r for r in records if r.stock > 50]
  out.write("report 10: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 10


def report_11(records, out):
  selected = [r for r in records if r.stock > 55]
  out.write("report 11: %d records\n" % len(selected))
  for record in selected:
    out.write("  %s (%s)\n" % (record.title, record.owner))
  return len(selected) + 11
