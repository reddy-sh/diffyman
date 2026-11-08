import csv
from collections import defaultdict


def load_totals(path):
    totals = defaultdict(float)
    with open(path, newline="") as handle:
        for row in csv.DictReader(handle):
            region = row["region"].strip().lower()
            totals[region] += float(row["amount"])
    return totals


def summarize(path):
    totals = load_totals(path)
    return "\n".join(
        f"{region}: {totals[region]:.2f}" for region in sorted(totals)
    )


if __name__ == "__main__":
    print(summarize("sales.csv"))
