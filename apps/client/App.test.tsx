// App shell, headless at phone width (390 x 844): language and navigation.
import { expect, open, test } from "nsh/test";

test("opens in Arabic with today's figures", async () => {
  const app = await open(".", { locale: "ar-DZ" });
  await app.waitFor("text:مبيعات اليوم");
  expect(app.texts()).toContain("الرئيسية");
  expect(app.texts()).toContain("الكريدي — من عليه دين");
});

test("switches language without restarting", async () => {
  const app = await open(".", { locale: "ar-DZ" });
  await app.waitFor("text:مبيعات اليوم");
  await app.press("label:اللغة");
  await app.waitFor("text:Ventes du jour");
  expect(app.texts()).toContain("Accueil");
  await app.press("label:Langue");
  await app.waitFor("text:Today's sales");
});

test("navigates between sections", async () => {
  const app = await open(".", { locale: "fr-DZ" });
  await app.waitFor("text:Ventes du jour");
  await app.press("text:Clients");
  await app.waitFor("text:Cet écran est en construction et arrive dans les prochaines étapes.");
  await app.press("text:Accueil");
  await app.waitFor("text:Ventes du jour");
});

test("adds a product from the phone, with validation", async () => {
  const app = await open(".", { locale: "fr-DZ" });
  await app.waitFor("text:Ventes du jour");
  await app.press("text:Plus");
  await app.press("text:Produits");
  await app.waitUntil(() => app.texts().some((t) => t.includes("produits")));
  await app.press("text:Ajouter un produit");
  await app.waitFor("text:Stock initial");
  await app.press("text:Enregistrer");
  await app.waitFor("text:Ce champ est obligatoire.");
  expect(app.texts()).toContain("Saisissez le prix en chiffres, par exemple 120 ou 120,50.");
  await app.type("id:product-name", "Lait test");
  await app.type("id:product-price", "120,5");
  await app.press("text:Enregistrer");
  await app.waitUntil(() => app.texts().some((t) => t.includes("Lait test") && t.includes("enregistré")));
  // The confirmation goes away by itself.
  await app.waitUntil(() => !app.texts().some((t) => t.includes("enregistré")), { timeout: 6000 });
  // The new product is first in the table.
  expect(app.texts().some((t) => t.includes("Lait test"))).toBe(true);
});

test("command palette: search across languages and run an action", async () => {
  const app = await open(".", { locale: "ar-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:مبيعات اليوم");
  await app.press("id:open-palette");
  await app.waitFor("text:انتقل إلى");
  // Typing French in the Arabic interface still finds the section.
  await app.type("label:ابحث في الأقسام والإجراءات", "stock");
  await app.waitFor("text:المخزون");
  await app.clear("label:ابحث في الأقسام والإجراءات");
  await app.press("text:أضف منتجاً");
  // The action goes to Products and opens the form straight away.
  await app.waitFor("text:المخزون الافتتاحي");
});
