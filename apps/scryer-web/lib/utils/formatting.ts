export function humanizeEntitlement(entitlement: string) {
  return entitlement.replace(/_/g, " ").replace(/\b\w/g, (char) => char.toUpperCase());
}
