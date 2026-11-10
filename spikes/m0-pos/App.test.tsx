// M0.6 spike: POS screen driven headless against the Rust root (`nsh.json` "root").
import { expect, open, test } from "nsh/test";

test("browses 10k products and searches in Arabic", async () => {
  const app = await open(".", { locale: "ar" });
  await app.waitFor("text:السلة");
  expect(app.texts().some((t) => t.startsWith("10000 نتيجة"))).toBe(true);

  await app.type("id:search", "حليب Ifri");
  await app.waitUntil(() => app.texts().some((t) => t.startsWith("200 نتيجة") || /^\d+ نتيجة/.test(t)));
  const stats = app.texts().find((t) => t.includes("نتيجة")) ?? "";
  expect(stats).not.toContain("10000 نتيجة");
});

test("adds products to the cart with integer totals", async () => {
  const app = await open(".", { locale: "ar" });
  await app.waitFor("text:السلة");
  await app.press("id:product >> nth=0");
  await app.press("id:product >> nth=0");
  await app.press("id:product >> nth=1");
  expect(app.texts()).toContain("3 قطعة");
});

test("scrolls the 10k grid", async () => {
  const app = await open(".", { locale: "ar" });
  await app.waitFor("text:السلة");
  await app.scroll("id:grid", 20000);
  await app.press("id:product >> nth=0");
  expect(app.texts()).toContain("1 قطعة");
});
