import csv


def summarize(path):
    totals = {}
    with open(path, newline="") as handle:
        for row in csv.DictReader(handle):
            region = row["region"].strip().lower()
            amount = float(row["amount"])
            if region not in totals:
                totals[region] = 0.0
            totals[region] += amount
    lines = []
    for region in sorted(totals):
        lines.append(f"{region}: {totals[region]:.2f}")
    return "\n".join(lines)


if __name__ == "__main__":
    print(summarize("sales.csv"))
