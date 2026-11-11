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

test("the owner adds a cashier with a PIN, locks the screen, and the cashier signs in", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  // No PIN or password on the device yet: no lock.
  expect(app.shows("id:lock-now")).toBe(false);
  await app.press("id:nav-settings");
  await app.press("id:settings-users");
  await app.waitFor("text:Propriétaire");
  await app.press("id:user-add");
  await app.waitFor("text:Code rapide (4 à 6 chiffres)");
  await app.type("id:user-name", "Karim Caisse");
  await app.type("id:user-pin", "1234");
  await app.press("id:user-save");
  // The rules come from the database: a guessable PIN is refused.
  await app.waitFor("text:Code trop facile à deviner (1234, 0000…). Choisissez-en un autre.");
  await app.clear("id:user-pin");
  await app.type("id:user-pin", "2580");
  await app.press("id:user-save");
  await app.waitFor("text:Karim Caisse");
  expect(app.texts()).toContain("Caissier");
  // Someone has a PIN now: the screen can be locked.
  await app.waitFor("id:lock-now");
  await app.press("id:lock-now");
  await app.waitFor("id:lock-screen");
  await app.press("text:Karim Caisse");
  await app.waitFor("id:pin-ok");
  for (const d of "1111") await app.press(`id:pin-${d}`);
  await app.press("id:pin-ok");
  await app.waitFor("text:Identifiants incorrects.");
  for (const d of "2580") await app.press(`id:pin-${d}`);
  await app.press("id:pin-ok");
  await app.waitFor("id:lock-now");
  // A cashier sees no accounting.
  expect(app.texts()).not.toContain("Comptabilité");
});

test("the activity decides which modules show", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  // A grocery has no delivery rounds.
  await app.waitUntil(() => !app.texts().includes("Distribution"));
  await app.press("id:nav-settings");
  await app.press("id:settings-store");
  await app.waitFor("text:Modules activés");
  await app.press("id:activity-select");
  await app.press("text:Commerce de gros");
  await app.press("id:store-save");
  await app.waitFor("text:Paramètres enregistrés");
  await app.waitFor("text:Distribution");
});

test("categories form a tree", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Catalogue");
  await app.press("id:nav-categories");
  await app.waitFor("text:Boissons");
  await app.press("id:category-add");
  await app.type("id:category-name", "Eaux");
  await app.press("id:category-parent");
  await app.press("text:Boissons");
  await app.press("id:category-save");
  await app.waitFor("text:Catégorie enregistrée : Boissons › Eaux");
  expect(app.texts()).toContain("Eaux");
});

test("importing products checks every line before writing", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Catalogue");
  await app.press("id:nav-products");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "2 000 résultats"));
  await app.press("id:products-import");
  // One line only (headless typing ends a line with the return key): no rows.
  await app.type("id:import-text", "Nom;Prix");
  await app.press("id:import-run");
  await app.waitFor("text:Le fichier n'a aucune ligne de produit.");
  await app.clear("id:import-text");
  await app.paste("id:import-text", { text: "Nom;Prix;Catégorie\nCafé moulu;450;Épicerie\nThé;abc;\n" });
  await app.press("id:import-run");
  await app.waitUntil(() =>
    app.texts().some((t) => plain(t) === "Ligne 3 : Saisissez le prix en chiffres, par exemple 120 ou 120,50."),
  );
  await app.clear("id:import-text");
  await app.paste("id:import-text", { text: "Nom;Prix;Catégorie\nCafé moulu;450;Épicerie\nThé vert;220;Épicerie\n" });
  await app.press("id:import-run");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "2 002 résultats"));
});
