export type ChoiceOption = { value: string; label: string; description?: string; group?: string };

export function browseChoices<T extends ChoiceOption>(options: readonly T[], query: string, locale = "en") {
  const normalize = (value: string) => value.normalize("NFKC").toLocaleLowerCase(locale).replace(/\+/g, " ");
  const words = normalize(query).trim().split(/\s+/).filter(Boolean);
  const keyQuery = words.some(word => ["ctrl", "alt", "shift"].includes(word));
  const groups: { label: string; options: T[] }[] = [];
  for (const option of options) {
    const haystack = normalize([option.label, option.description, option.group].filter(Boolean).join(" "));
    if (!words.every(word => keyQuery && /^[a-z0-9]$/.test(word)
      ? haystack.split(/\s+/).includes(word) : haystack.includes(word))) continue;
    const label = option.group || "";
    let group = groups.find(item => item.label === label);
    if (!group) { group = { label, options: [] }; groups.push(group); }
    group.options.push(option);
  }
  return groups;
}
