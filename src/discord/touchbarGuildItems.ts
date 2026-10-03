export function replaceGuildItems<T>(items: T[], replacements: readonly T[]): void {
    items.splice(0, items.length, ...replacements);
}
