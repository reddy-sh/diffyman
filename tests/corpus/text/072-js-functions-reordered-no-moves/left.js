function formatDate(date) {
  return date.toISOString().slice(0, 10);
}

function formatMoney(cents, currency) {
  return (cents / 100).toFixed(2) + " " + currency;
}

function formatPercent(ratio) {
  return Math.round(ratio * 100) + "%";
}
