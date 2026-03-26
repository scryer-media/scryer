export function humanizeEnumValue(value: string) {
  return value.replace(/_/g, " ").replace(/\b\w/g, (char) => char.toUpperCase());
}

export function humanizeEntitlement(entitlement: string) {
  return humanizeEnumValue(entitlement);
}
