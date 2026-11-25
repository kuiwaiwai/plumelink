export type FileOrder = "default" | "name" | "largest";
export function browseFiles<T extends { name: string; size: number }>(
  files: readonly T[], query: string, order: FileOrder, locale: string,
): T[] {
  const needle = query.normalize("NFC").toLocaleLowerCase(locale).trim();
  const result = files.filter(file => file.name.normalize("NFC").toLocaleLowerCase(locale).includes(needle));
  if (order === "default") return result;
  const collator = new Intl.Collator(locale, { numeric: true, sensitivity: "base" });
  return result.sort((a, b) => order === "largest"
    ? b.size - a.size || collator.compare(a.name, b.name)
    : collator.compare(a.name, b.name));
}
