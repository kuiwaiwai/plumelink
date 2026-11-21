import { afterEach, describe, expect, it } from "vitest";
import { displayMessage, languagePreference, resolveLocale, t } from "../shared/i18n";
import english from "../shared/locales/en.json";
import traditional from "../shared/locales/zh-Hant.json";

describe("interface languages", () => {
  afterEach(() => { languagePreference.value = "system"; });

  it("respects language order, Chinese scripts and regional defaults", () => {
    expect(resolveLocale(["fr", "zh-HK", "en"])).toBe("zh-TW");
    expect(resolveLocale(["zh-Hans-TW"])).toBe("zh-CN");
    expect(resolveLocale(["zh-Hant-CN"])).toBe("zh-TW");
    expect(resolveLocale(["en-GB", "zh-CN"])).toBe("en");
    expect(resolveLocale(["?", "fr"])).toBe("en");
  });

  it("preserves every message parameter in both translations", () => {
    const parameters = (text: string) => text.match(/\{\d+\}/g)?.sort() ?? [];
    expect(Object.keys(traditional).sort()).toEqual(Object.keys(english).sort());
    for (const key of Object.keys(english) as (keyof typeof english)[]) {
      expect(parameters(english[key]), key).toEqual(parameters(key));
      expect(parameters(traditional[key]), key).toEqual(parameters(key));
    }
  });

  it("preserves names and OS details when formatting messages", () => {
    languagePreference.value = "en";
    expect(t("选择 {0}", "文件 {0}.txt")).toBe("Select 文件 {0}.txt");
    expect(displayMessage("无法读取所选文件：用户 {0}.txt")).toBe("Could not read selected file: 用户 {0}.txt");
    expect(displayMessage("driver-specific error")).toBe("driver-specific error");
  });
});
