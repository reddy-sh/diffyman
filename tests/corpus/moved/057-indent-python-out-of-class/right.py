def slugify(text):
  cleaned = "".join(ch if ch.isalnum() else "-" for ch in text.lower())
  return "-".join(part for part in cleaned.split("-") if part)


class Article:
  def __init__(self, title):
    self.title = title

  def url(self):
    return "/articles/" + slugify(self.title)
