// 🎉 greeting helpers
export function greet(user) {
  const label = "👋 " + user.name;
  return label + " (" + user.locale + ")";
}
