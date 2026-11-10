// App shell, headless at phone width (390 x 844): language and navigation.
import { expect, open, test } from "nsh/test";

test("opens in Arabic with the overview", async () => {
  const app = await open(".", { locale: "ar-DZ" });
  await app.waitFor("text:إجمالي المبيعات");
  expect(app.texts()).toContain("الرئيسية");
  expect(app.texts()).toContain("ديون الزبائن");
  expect(app.texts()).toContain("آخر الفواتير");
});

test("switches language from the settings, without restarting", async () => {
  const app = await open(".", { locale: "ar-DZ" });
  await app.waitFor("text:إجمالي المبيعات");
  await app.press("text:المزيد");
  await app.press("text:الإعدادات");
  await app.waitFor("text:استعادة الافتراضي");
  await app.press("label:اللغة");
  await app.press("text:Français");
  await app.waitFor("text:Rétablir les valeurs par défaut");
  expect(app.texts()).toContain("Paramètres");
  await app.press("id:bottom-home");
  await app.waitFor("text:Ventes brutes");
});

test("navigates between sections", async () => {
  const app = await open(".", { locale: "fr-DZ" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Clients");
  await app.waitFor("text:Cet écran est en construction et arrive dans les prochaines étapes.");
  await app.press("text:Accueil");
  await app.waitFor("text:Ventes brutes");
});

test("adds a product from the phone, with validation", async () => {
  const app = await open(".", { locale: "fr-DZ" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Plus");
  await app.press("text:Produits");
  await app.waitFor("text:Ajouter un produit");
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
  await app.waitFor("text:إجمالي المبيعات");
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

/** Visible text without bidi isolates and with plain spaces, for matching figures. */
const plain = (t: string) => t.replace(/[\u2066-\u2069]/g, "").replace(/\u00a0/g, " ");

test("products: status tabs filter the table and clearing restores it", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const shows = (s: string) => () => app.texts().some((t) => plain(t) === s);
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Catalogue");
  await app.press("id:nav-products");
  await app.waitUntil(shows("2 000 résultats"));
  await app.press("id:tab-3");
  await app.waitUntil(shows("72 résultats"));
  await app.type("id:product-search", "zzz");
  await app.waitFor("text:Aucun résultat");
  await app.press("text:Effacer");
  await app.waitUntil(shows("2 000 résultats"));
});

test("appearance settings apply at once and keep the page", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-settings");
  await app.waitFor("text:Rétablir les valeurs par défaut");
  await app.press("id:settings-appearance");
  await app.press("label:Émeraude");
  await app.waitFor("text:Émeraude");
  await app.press("text:Compacte");
  await app.press("text:Grand");
  await app.press("text:Sombre");
  // The interface was rebuilt with the new look; the user is still on the settings.
  await app.waitFor("text:Rétablir les valeurs par défaut");
  await app.press("text:Rétablir les valeurs par défaut");
  await app.waitFor("text:Indigo");
  await app.press("text:Accueil");
  await app.waitFor("text:Ventes brutes");
});

test("sidebar sections expand to their pages, and settings tabs switch panels", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  expect(app.texts()).not.toContain("Plan comptable");
  await app.press("text:Comptabilité");
  await app.waitFor("text:Plan comptable");
  await app.press("text:Plan comptable");
  await app.waitFor("text:Arrive dans une prochaine étape");
  // The section's other pages are tabs on the page.
  await app.press("id:tab-ledger");
  await app.waitUntil(() => app.texts().filter((t) => t === "Grand livre").length >= 2);
  await app.press("id:nav-settings");
  await app.press("id:settings-taxes");
  await app.waitFor("text:Droit de timbre sur les paiements en espèces");
  await app.press("id:settings-store");
  await app.waitFor("text:Identifiants légaux");
});

test("the database's rules come back under their fields", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Catalogue");
  await app.press("id:nav-products");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "2 000 résultats"));
  await app.press("text:Ajouter un produit");
  await app.waitFor("text:Stock initial");
  await app.type("id:product-name", "Doublon");
  await app.type("id:product-price", "10");
  // P-01000 is the first demo product's code.
  await app.type("id:product-sku", "p-01000");
  await app.press("text:Enregistrer");
  await app.waitFor("text:Un autre produit a déjà ce code.");
  await app.clear("id:product-sku");
  await app.type("id:product-sku", "DBL-1");
  await app.type("id:product-stock", "1,5");
  await app.press("text:Enregistrer");
  // A piece is counted in whole units.
  await app.waitFor("text:Saisissez une quantité, par exemple 0, 24 ou 1,5 pour les kilos.");
  await app.clear("id:product-stock");
  await app.type("id:product-stock", "3");
  await app.press("text:Enregistrer");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "2 001 résultats"));
  await app.type("id:product-search", "doublon");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1 résultat"));
});
