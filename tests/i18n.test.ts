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
  it("translates window-list timeouts when the interface language changes", () => {
    languagePreference.value = "en";
    const message = displayMessage("窗口列表超时");
    expect(message).toBe("The window list timed out. Refresh to retry.");
    languagePreference.value = "zh-TW";
    expect(displayMessage(message)).toBe("讀取視窗清單逾時，請重新整理後重試。");
    languagePreference.value = "zh-CN";
    expect(displayMessage(message)).toBe("窗口列表超时");
  });
  it("updates stored failure details and their uncertainty warning when language changes", () => {
    const reason = "此应用需要管理员权限，请在电脑端启动";
    const warning = "结果未确认，请检查电脑；不会自动重发";
    const saved = reason + "\n" + warning;
    languagePreference.value = "en";
    const englishMessage = displayMessage(saved);
    expect(englishMessage).toBe(english[reason] + "\n" + english[warning]);
    languagePreference.value = "zh-TW";
    expect(displayMessage(englishMessage)).toBe(traditional[reason] + "\n" + traditional[warning]);
    expect(displayMessage(reason + "\nprivate detail")).toBe(reason + "\nprivate detail");
  });
});
