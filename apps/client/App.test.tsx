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
  await app.waitFor("text:Qui achète, ce qu'ils doivent, et le crédit accordé.");
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
  // The editor: organized in tabs, saved from its header.
  await app.waitFor("text:Identification", { timeout: 15000 });
  await app.press("id:product-save");
  await app.waitFor("text:Ce champ est obligatoire.");
  await app.type("id:product-name", "Lait test");
  await app.press("id:editor-tab-prices");
  await app.waitFor("text:Prix de vente");
  await app.type("id:product-price", "120,5");
  await app.press("id:product-save");
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
  await app.waitFor("text:التعريف");
});

/** Visible text without bidi isolates and with plain spaces, for matching figures. */
const plain = (t: string) => t.replace(/[\u2066-\u2069]/g, "").replace(/\u00a0/g, " ");

test("products: status tabs filter the table and clearing restores it", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const shows = (s: string) => () => app.texts().some((t) => plain(t) === s);
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Catalogue");
  await app.press("id:nav-products");
  await app.waitUntil(shows("1–25 sur 2 000"));
  await app.press("id:tab-3");
  await app.waitUntil(shows("1–25 sur 72"));
  await app.type("id:product-search", "zzz");
  await app.waitFor("text:Aucun résultat");
  await app.press("text:Effacer");
  await app.waitUntil(shows("1–25 sur 2 000"));
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
  expect(app.texts()).not.toContain("Dépenses");
  await app.press("text:Finances");
  await app.waitFor("text:Dépenses");
  await app.press("text:Caisses");
  await app.waitFor("text:Arrive dans une prochaine étape");
  // The section's other pages are tabs on the page.
  await app.press("id:tab-banks");
  await app.waitUntil(() => app.texts().filter((t) => t === "Banques").length >= 2);
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
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 2 000"));
  await app.press("text:Ajouter un produit");
  await app.waitFor("text:Identification", { timeout: 15000 });
  await app.type("id:product-name", "Doublon");
  // P-01000 is the first demo product's code.
  await app.type("id:product-sku", "p-01000");
  await app.press("id:editor-tab-prices");
  await app.type("id:product-price", "10");
  await app.press("id:product-save");
  // The error opens the tab of its field.
  await app.waitFor("text:Un autre produit a déjà ce code.");
  await app.clear("id:product-sku");
  await app.type("id:product-sku", "DBL-1");
  await app.press("id:editor-tab-stock");
  await app.type("id:product-stock", "1,5");
  await app.press("id:product-save");
  // A piece is counted in whole units.
  await app.waitFor("text:Cette unité n'accepte pas autant de décimales.");
  await app.clear("id:product-stock");
  await app.type("id:product-stock", "3");
  await app.press("id:product-save");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 2 001"));
  await app.type("id:product-search", "doublon");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–1 sur 1"));
});

test("the editor generates a barcode, packaging, list prices and variants", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Catalogue");
  await app.press("id:nav-products");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 2 000"));
  await app.press("text:Ajouter un produit");
  await app.waitFor("text:Identification", { timeout: 15000 });
  await app.type("id:product-name", "T-shirt coton");
  await app.type("id:product-sku", "TSH");
  await app.press("id:editor-tab-prices");
  await app.type("id:product-price", "1500");
  await app.type("id:product-cost", "900");
  // The margin follows what is typed.
  await app.waitUntil(() => app.texts().some((t) => plain(t).startsWith("600,00 DA")));
  await app.press("id:price-row-add");
  await app.type("id:price-row-qty-0", "10");
  await app.type("id:price-row-price-0", "1400");
  await app.press("id:editor-tab-units");
  await app.press("id:barcode-generate-0");
  await app.waitUntil(() => app.texts().some((t) => t.startsWith("2000000000")));
  await app.press("id:pack-add");
  await app.type("id:pack-factor-0", "12");
  await app.press("id:editor-tab-variants");
  await app.press("id:attr-add");
  await app.type("id:attr-name-0", "Taille");
  await app.type("id:attr-values-0", "S, M, L");
  await app.waitFor("id:variants-preview");
  await app.press("id:product-save");
  await app.waitUntil(() => app.texts().some((t) => t.includes("T-shirt coton") && t.includes("enregistré")));
  await app.type("id:product-search", "t-shirt coton");
  // The product and its three variants.
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–4 sur 4"));
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
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 2 000"));
  await app.press("id:products-import");
  // One line only (headless typing ends a line with the return key): no rows.
  await app.type("id:import-text", "Nom;Prix");
  await app.press("id:import-run");
  await app.waitFor("text:Le fichier n'a aucune ligne de produit.");
  await app.clear("id:import-text");
  await app.paste("id:import-text", { text: "Nom;Prix;Catégorie\nCafé moulu;450;Épicerie\nThé;abc;\n" });
  await app.press("id:import-run");
  // The faulty line is marked in the preview, with why; nothing is written.
  await app.waitUntil(() =>
    app.texts().some((t) => plain(t).includes("Saisissez le prix en chiffres, par exemple 120 ou 120,50.")),
  );
  await app.waitFor("id:import-errors");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1 en erreur"));
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1 valides"));
  // The good lines only, when asked.
  await app.press("id:import-skip-bad");
  await app.press("id:import-run");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 2 001"));
  await app.press("id:products-import");
  await app.paste("id:import-text", { text: "Nom;Prix;Catégorie\nThé vert;220;Épicerie\nThé noir;240;Épicerie\n" });
  await app.press("id:import-run");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 2 003"));
});

test("tax classes and scale labels are the store's settings", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-settings");
  await app.press("id:settings-taxes");
  await app.waitFor("text:Classes de TVA");
  await app.waitFor("id:tax-rate-1", { timeout: 10000 });
  await app.clear("id:tax-rate-1");
  await app.type("id:tax-rate-1", "9,5");
  await app.press("id:tax-save");
  await app.waitFor("text:Paramètres enregistrés");
  await app.press("id:settings-devices");
  await app.waitFor("text:Étiquettes de balance");
  await app.press("id:scale-add");
  await app.waitUntil(() => app.texts().some((t) => plain(t).includes("article 42, prix 180")));
  await app.press("id:scale-save");
  await app.waitFor("text:Paramètres enregistrés");
});

test("stock levels and a stock count", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Stock");
  await app.press("id:nav-inventory");
  await app.waitFor("text:Valeur du stock");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 2 000"));
  await app.press("id:stock-tab-2");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 129"));
  // A count: 5 units of the first product found, then closed.
  await app.press("id:tab-stock_counts");
  await app.press("id:count-open");
  await app.press("id:count-start");
  await app.waitFor("text:Comptage");
  await app.clear("id:count-qty");
  await app.type("id:count-qty", "5");
  await app.type("id:count-search", "P-01000");
  await app.press("id:count-search-result-0");
  await app.waitFor("id:count-line-0");
  await app.press("id:count-close");
  await app.waitUntil(() => app.texts().some((t) => plain(t).startsWith("Inventaire clôturé : 1")));
  await app.waitFor("text:Clôturé");
  await app.press("id:tab-movements");
  await app.waitUntil(() => app.texts().some((t) => t.includes("INV-01-26")));
});

test("a manual entry, its reversal, and the trial balance", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Comptabilité");
  await app.press("id:nav-journal");
  await app.waitFor("id:entry-new");
  await app.press("id:entry-new");
  await app.type("id:entry-memo", "Apport en caisse");
  await app.press("id:entry-account-0");
  await app.press("text:531 — Caisse principale");
  await app.type("id:entry-debit-0", "1500");
  await app.press("id:entry-account-1");
  await app.press("text:101 — Capital");
  await app.type("id:entry-credit-1", "1400");
  await app.press("id:entry-post");
  await app.waitFor("text:Les débits doivent égaler les crédits.");
  await app.clear("id:entry-credit-1");
  await app.type("id:entry-credit-1", "1500");
  await app.waitFor("text:Équilibrée");
  await app.press("id:entry-post");
  await app.waitUntil(() => app.texts().some((t) => /^Écriture OD-01-\d\d-A\d+ passée$/.test(plain(t))));
  await app.waitFor("text:Apport en caisse");
  // Reversed, it cancels itself out.
  await app.press("id:entry-reverse-0");
  await app.type("id:reverse-reason", "doublon");
  await app.press("id:reverse-confirm");
  await app.waitUntil(() => app.texts().some((t) => t.startsWith("Contre-passée par")));
  await app.press("id:tab-trial_balance");
  await app.waitFor("text:Équilibrée");
  await app.waitFor("id:trial-531");
  await app.press("id:tab-accounts");
  await app.waitFor("text:Caisses et banques");
  await app.waitFor("id:account-531");
});

test("the products table pages through the catalog", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Catalogue");
  await app.press("id:nav-products");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 2 000"));
  await app.press("id:page-next");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "26–50 sur 2 000"));
  await app.press("id:page-80");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1 976–2 000 sur 2 000"));
  // A filter starts again from the first page.
  await app.press("id:tab-2");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 129"));
  await app.press("id:page-size");
  await app.press("text:100");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–100 sur 129"));
});

test("a cash sale at the till, from opening to closing", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "100");
  await app.press("id:till-open");
  await app.waitFor("id:pos-search");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-item-1");
  await app.waitFor("id:cart-line-1");
  await app.press("id:cart-less-0");
  await app.waitUntil(() => !app.texts().some((t) => plain(t).startsWith("2 × ")));
  // Tap the quantity: type it, change the price and the name.
  await app.press("id:cart-edit-0");
  await app.waitFor("id:line-apply");
  await app.clear("id:line-qty");
  await app.type("id:line-qty", "3");
  await app.clear("id:line-price");
  await app.type("id:line-price", "100");
  await app.clear("id:line-name");
  await app.type("id:line-name", "Biscuits en promo");
  await app.press("id:line-apply");
  await app.waitFor("text:Biscuits en promo");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "3 × 100,00 DA"));
  // An item not in the catalog.
  await app.press("id:pos-free");
  await app.type("id:free-name", "Sac cabas");
  await app.type("id:free-price", "20");
  await app.press("id:free-add");
  await app.waitFor("text:Sac cabas");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.clear("id:pay-given");
  await app.type("id:pay-given", "5000");
  await app.press("id:pay-confirm");
  await app.waitUntil(() => app.texts().some((t) => /^Vente FV-01-\d\d-A\d+ enregistrée — rendre /.test(plain(t))));
  await app.waitFor("text:Scannez ou touchez un produit pour commencer la vente.");
  // Closing: the drawer is counted right.
  await app.press("id:till-close");
  await app.waitFor("text:Attendu dans le tiroir");
  await app.type("id:till-counted", "0");
  await app.waitUntil(() => app.texts().some((t) => plain(t).startsWith("Manquant de ")));
  await app.press("id:till-close-confirm");
  await app.waitFor("id:till-open");
});

test("a cart put aside comes back, and a sale is returned", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "1000");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.waitFor("id:cart-line-0");
  // Put aside: the till is free; the cart waits under its number.
  await app.press("id:cart-hold");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "Panier mis de côté : #1"));
  await app.waitFor("text:Scannez ou touchez un produit pour commencer la vente.");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "En attente (1)"));
  await app.press("id:pos-held");
  await app.waitFor("id:held-0");
  await app.press("id:held-0");
  await app.waitFor("id:cart-line-0");
  await app.waitUntil(() => !app.texts().some((t) => plain(t).startsWith("En attente (")));
  // Sold, then brought back.
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.press("id:pay-confirm");
  let number = "";
  await app.waitUntil(() =>
    app.texts().some((t) => {
      const m = /^Vente (FV-01-\d\d-A\d+) enregistrée/.exec(plain(t));
      if (m) number = m[1];
      return !!m;
    }),
  );
  await app.press("id:pos-return");
  await app.waitFor("id:ret-sale");
  await app.type("id:ret-sale", number);
  await app.press("id:ret-find");
  await app.waitFor("id:ret-all-0");
  await app.press("id:ret-all-0");
  await app.type("id:ret-reason", "Emballage abîmé");
  await app.press("id:ret-confirm");
  await app.waitUntil(() => app.texts().some((t) => /^Retour AV-01-\d\d-A\d+ enregistré : .+ remboursés\.$/.test(plain(t))));
  // Nothing is left to return.
  await app.press("id:pos-return");
  await app.type("id:ret-sale", number);
  await app.press("id:ret-find");
  await app.waitFor("text:Entièrement retourné");
});

test("a quote becomes an order, invoiced at the till with its discount", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Ventes");
  await app.press("id:nav-quotes");
  await app.waitFor("id:doc-new-quote");
  await app.waitFor("text:Aucun document");
  await app.press("id:doc-new-quote");
  await app.type("id:doc-customer", "Boulangerie El Amel");
  await app.type("id:doc-search", "Biscuits");
  await app.press("id:doc-search-result-0");
  await app.waitFor("id:doc-qty-0");
  await app.clear("id:doc-qty-0");
  await app.type("id:doc-qty-0", "10");
  await app.type("id:doc-discount", "10");
  await app.press("id:doc-save");
  await seen(/^DV-01-\d\d-A\d+ enregistré\.$/);
  // The draft is approved, then turned into an order.
  await app.press("id:doc-approve");
  await seen(/^DV-01-\d\d-A\d+ validé\.$/);
  await app.press("id:doc-convert");
  await seen(/^Commande BC-01-\d\d-A\d+ créée depuis le devis\.$/);
  await app.waitFor("id:doc-approve");
  await app.press("id:doc-approve");
  await app.waitFor("id:doc-invoice");
  await app.press("id:doc-invoice");
  // At the till: the order fills the cart, with its discount.
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await seen(/^Facturation de BC-01-\d\d-A\d+$/);
  await app.waitFor("id:cart-line-0");
  await seen(/^Remise 10 %$/);
  await app.press("id:pos-discount");
  await app.waitFor("id:disc-apply");
  await app.press("id:disc-15");
  await app.press("id:disc-apply");
  await seen(/^Remise 15 %$/);
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.press("id:pay-confirm");
  await seen(/^Vente FV-01-\d\d-A\d+ enregistrée/);
  // The sale lists the order; the order is invoiced.
  await app.press("id:nav-sales");
  await app.waitFor("id:sale-0");
  await seen(/^BC-01-\d\d-A\d+$/);
  await app.press("id:nav-quotes");
  await app.waitFor("id:doc-0");
  await app.waitFor("text:Facturé");
  await app.waitFor("text:Transformé en commande");
  // The database was checked at start and found sound.
  expect(app.texts()).not.toContain("Contrôle des données en échec");
});

test("a customer buys on account, and owes it", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Clients et crédit");
  await app.press("id:nav-customers");
  await app.waitFor("id:customer-new");
  await app.waitFor("text:Boulangerie El Amel");
  await app.press("id:customer-new");
  await app.type("id:customer-name", "Épicerie Test");
  await app.type("id:customer-phone", "0770 11 22 33");
  await app.type("id:customer-limit", "5000");
  await app.press("id:customer-save");
  await seen(/^Épicerie Test enregistré\.$/);
  // At the till: the customer pays part, the rest goes on the account.
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-customer");
  await app.type("id:cust-search", "0770");
  await app.waitFor("id:cust-pick-0");
  await app.press("id:cust-pick-0");
  await app.waitFor("text:Épicerie Test");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.clear("id:pay-given");
  await app.type("id:pay-given", "100");
  await app.waitFor("text:Sur compte");
  await app.press("id:pay-confirm");
  await seen(/^Vente FV-01-\d\d-A\d+ enregistrée/);
  // The customer now owes the rest.
  await app.press("id:nav-customers");
  await app.press("text:Débiteurs");
  await app.waitFor("text:Épicerie Test");
  await app.waitUntil(() => app.texts().filter((t) => t === "Épicerie Test").length === 1 && app.texts().includes("Doit"));
  // The debts by age list it; its card collects the payment into the till.
  await app.press("id:tab-debts");
  await app.waitFor("id:debt-0");
  await app.waitFor("text:0–30 jours");
  await app.press("id:debt-0");
  await app.waitFor("id:card-collect");
  await app.waitFor("text:Relevé");
  await app.waitFor("text:Vente");
  await app.press("id:card-collect");
  await app.waitFor("id:rc-confirm");
  await app.press("id:rc-confirm");
  await seen(/^Reçu RC-01-\d\d-A\d+ : .+ encaissés\.$/);
  await app.waitFor("text:Encaissement");
  await app.waitFor("text:Personne ne vous doit rien");
});

test("a cashier's large discount is approved by a supervisor's PIN", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-settings");
  await app.press("id:settings-users");
  await app.waitFor("text:Propriétaire");
  // A store manager who may approve, and a cashier.
  await app.press("id:user-add");
  await app.waitFor("id:user-name");
  await app.type("id:user-name", "Sara Gérante");
  await app.type("id:user-pin", "3691");
  await app.press("text:Responsable de magasin");
  await app.press("id:user-save");
  await app.waitFor("text:Sara Gérante");
  await app.waitUntil(() => !app.shows("id:user-name"));
  await new Promise((r) => setTimeout(r, 500));
  await app.press("id:user-add");
  await app.waitFor("id:user-name");
  await app.type("id:user-name", "Karim Caisse");
  await app.type("id:user-pin", "2580");
  await app.press("id:user-save");
  await app.waitFor("text:Karim Caisse");
  // The cashier signs in and sells with 20 % off a line.
  await app.press("id:lock-now");
  await app.waitFor("id:lock-screen");
  await app.waitUntil(() => app.texts().includes("Karim Caisse"), { timeout: 10000 });
  await app.press("text:Karim Caisse");
  await app.waitFor("id:pin-ok");
  for (const d of "2580") await app.press(`id:pin-${d}`);
  await app.press("id:pin-ok");
  await app.waitFor("id:lock-now");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:cart-edit-0");
  await app.waitFor("id:line-discount");
  await app.type("id:line-discount", "20");
  await app.press("id:line-apply");
  await app.waitUntil(() => !app.shows("id:line-apply"));
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.press("id:pay-confirm");
  // A supervisor approves on the spot; a wrong PIN is refused.
  await app.waitFor("id:approval-confirm");
  await seen(/^Il faut l'accord d'un responsable : une remise au-delà de l'habituel\.$/);
  await app.type("id:approval-pin", "1478");
  await app.press("id:approval-confirm");
  await app.waitFor("text:Identifiants incorrects.");
  await app.clear("id:approval-pin");
  await app.type("id:approval-pin", "3691");
  await app.press("id:approval-confirm");
  await seen(/^Vente FV-01-\d\d-A\d+ enregistrée/);
});

test("an invoice on account is split into instalments and the first is paid", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-customer");
  await app.type("id:cust-search", "Restaurant");
  await app.waitFor("id:cust-pick-0");
  await app.press("id:cust-pick-0");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.clear("id:pay-given");
  await app.type("id:pay-given", "0");
  await app.press("id:pay-confirm");
  await seen(/^Vente FV-01-\d\d-A\d+ enregistrée/);
  // The customer's card: split the invoice in three monthly instalments.
  await app.press("text:Clients et crédit");
  await app.press("id:nav-customers");
  await app.type("id:customers-search", "Restaurant");
  await app.waitFor("id:customer-0");
  await app.press("id:customer-0");
  await app.waitFor("text:Factures ouvertes");
  await app.press("text:Factures ouvertes");
  await app.waitFor("id:inv-plan-0");
  await app.press("id:inv-plan-0");
  await app.waitFor("id:plan-save");
  await app.waitFor("text:Échéances égales au centime ; les centimes restants vont aux premières.");
  await app.press("id:plan-save");
  await seen(/^Échéancier TQ-00001 créé\.$/);
  await app.waitFor("id:plan-pay");
  await app.waitFor("text:En cours");
  // The next instalment is paid in cash.
  await app.press("id:plan-pay");
  await app.waitFor("id:plan-pay-confirm");
  await app.press("id:plan-pay-confirm");
  await seen(/^Reçu RC-01-\d\d-A\d+ : .+ encaissés\.$/);
  await app.waitFor("text:Payé");
  await app.waitUntil(() => app.texts().filter((t) => t === "À venir").length === 2);
});

test("an expense from the bank, and a customer's opening balance", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Finances");
  await app.press("id:nav-expenses");
  await app.waitFor("id:expense-new");
  await app.waitFor("text:Rien d'enregistré");
  await app.press("id:expense-new");
  await app.waitFor("id:exp-save");
  // No till open: the bank pays.
  await app.waitFor("text:Aucune caisse ouverte : banque seulement.");
  await app.type("id:exp-total-input", "1500");
  await app.type("id:exp-payee", "Sonelgaz");
  await app.press("id:exp-save");
  await seen(/^DP-01-\d\d-A\d+ enregistré : 1 500,00 DA\.$/);
  await app.waitFor("id:expense-0");
  await seen(/^Divers · Sonelgaz$/);
  // A customer's debt from the old credit book.
  await app.press("text:Clients et crédit");
  await app.press("id:nav-customers");
  await app.waitFor("id:customer-0");
  await app.press("id:customer-0");
  await app.waitFor("id:card-adjust");
  await app.press("id:card-adjust");
  await app.waitFor("id:adj-confirm");
  await app.waitFor("text:Ce que le client devait avant cette application (cahier de crédit).");
  await app.type("id:adj-amount", "200");
  await app.type("id:adj-reason", "Cahier de crédit");
  await app.press("id:adj-confirm");
  await seen(/^Enregistré\. Le client doit maintenant 200,00 DA\.$/);
  await app.waitFor("text:Solde d'ouverture");
});

test("a supplier's invoice comes into stock, and the supplier is paid", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Achats");
  await app.press("id:nav-suppliers");
  await app.waitFor("text:Aucun fournisseur");
  await app.waitFor("id:supplier-new");
  await app.press("id:supplier-new");
  await app.waitFor("id:supplier-save");
  await app.type("id:supplier-name", "Cevital");
  await app.press("id:supplier-save");
  await app.waitFor("id:supplier-0");
  // Ten at 100.00 before tax, nothing paid now: all of it is owed.
  await app.press("id:nav-purchases");
  await app.waitFor("id:purchase-new");
  await app.press("id:purchase-new");
  await app.waitFor("id:buy-save");
  await app.type("id:buy-search", "Biscuits");
  await app.press("id:buy-search-result-0");
  await app.waitFor("id:buy-cost-0");
  await app.clear("id:buy-qty-0");
  await app.type("id:buy-qty-0", "10");
  await app.type("id:buy-cost-0", "100");
  await app.type("id:buy-ref", "FC-118");
  await app.press("id:buy-save");
  await seen(/^Achat FA-.+ enregistré : 1 190,00 DA\.$/);
  await app.waitFor("id:purchase-0");
  await seen(/^1 190,00 DA à payer$/);
  // The supplier is paid in full by bank.
  await app.press("id:nav-suppliers");
  await app.waitFor("id:supplier-0");
  await app.press("id:supplier-0");
  await app.waitFor("id:supplier-pay");
  await app.press("id:supplier-pay");
  await app.waitFor("id:sp-confirm");
  await app.press("id:sp-confirm");
  await seen(/^Paiement .+ : 1 190,00 DA payé\.$/);
});

test("a purchase order is approved, then received in part", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Achats");
  await app.press("id:nav-suppliers");
  await app.waitFor("id:supplier-new");
  await app.press("id:supplier-new");
  await app.waitFor("id:supplier-save");
  await app.type("id:supplier-name", "Cevital");
  await app.press("id:supplier-save");
  await app.waitFor("id:supplier-0");
  // Ten at 100.00 before tax, as a draft.
  await app.press("id:nav-purchase_orders");
  await app.waitFor("id:order-new");
  await app.waitFor("text:Aucun bon de commande");
  await app.press("id:order-new");
  await app.waitFor("id:order-save");
  await app.type("id:order-search", "Biscuits");
  await app.press("id:order-search-result-0");
  await app.waitFor("id:order-cost-0");
  await app.clear("id:order-qty-0");
  await app.type("id:order-qty-0", "10");
  await app.type("id:order-cost-0", "100");
  await app.press("id:order-save");
  await seen(/^Commande BCF-.+ enregistrée en brouillon\.$/);
  await app.press("text:Brouillons");
  await app.waitFor("id:order-0");
  await app.press("id:order-0");
  await app.waitFor("id:order-approve");
  await app.press("id:order-approve");
  await seen(/^Commande BCF-.+ approuvée\.$/);
  // Four came: received at the cost ordered.
  await app.waitFor("id:order-receive");
  await app.press("id:order-receive");
  await app.waitFor("id:buy-qty-0");
  await seen(/^Reste à recevoir : 10$/);
  await app.clear("id:buy-qty-0");
  await app.type("id:buy-qty-0", "4");
  await app.press("id:buy-save");
  await seen(/^Achat FA-.+ enregistré : 476,00 DA\.$/);
  await app.press("text:À recevoir");
  await app.waitFor("id:order-0");
  await app.waitFor("text:Reçu en partie");
});

test("goods brought back pay for others in one exchange", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "1000");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.press("id:pay-confirm");
  let number = "";
  await app.waitUntil(() =>
    app.texts().some((t) => {
      const m = /^Vente (FV-01-\d\d-A\d+) enregistrée/.exec(plain(t));
      if (m) number = m[1];
      return !!m;
    }),
  );
  // One comes back, toward another product.
  await app.press("id:pos-return");
  await app.waitFor("id:ret-sale");
  await app.type("id:ret-sale", number);
  await app.press("id:ret-find");
  await app.waitFor("id:ret-qty-0");
  await app.type("id:ret-qty-0", "1");
  await app.press("text:Échange");
  await app.type("id:ret-reason", "Mauvais parfum");
  await app.press("id:ret-confirm");
  await seen(/^La marchandise sera reprise au paiement de ce panier/);
  await app.waitFor("id:pos-exchange");
  await seen(new RegExp(`^Repris de ${number}$`));
  await app.press("id:pos-item-1");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-exchange");
  await app.press("id:pay-confirm");
  await seen(/^Échange fait : retour AV-01-\d\d-A\d+, vente FV-01-\d\d-A\d+\. À rendre : .+\.$/);
  await app.waitUntil(() => !app.texts().some((t) => plain(t).startsWith("Repris de ")));
});

test("a sale shows on the dashboard and in the reports", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.press("id:pay-confirm");
  let number = "";
  await app.waitUntil(() =>
    app.texts().some((t) => {
      const m = /^Vente (FV-01-\d\d-A\d+) enregistrée/.exec(plain(t));
      if (m) number = m[1];
      return !!m;
    }),
  );
  // The dashboard lists it.
  await app.press("id:nav-home");
  await app.waitFor("id:home-sale-0");
  await seen(new RegExp(`^${number}$`));
  // The report counts it, with its profit; by product, the two items.
  await app.press("text:Rapports");
  await app.press("id:nav-reports");
  await app.waitFor("id:rep-sales");
  await app.waitFor("id:rep-profit");
  await app.press("text:Par produit");
  await app.waitFor("id:rep-row-0");
  await seen(/^2$/);
  // Stock: its value at cost.
  await app.press("id:nav-stock_reports");
  await app.waitFor("id:stk-value");
  await app.waitFor("id:stk-row-0");
});

test("the finance reports show the result, the VAT and the till sessions", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.press("id:pay-confirm");
  await app.waitUntil(() => app.texts().some((t) => /^Vente FV-/.test(plain(t))));
  await app.press("text:Rapports");
  await app.press("id:nav-finance_reports");
  await app.waitFor("id:fin-result");
  await app.waitFor("id:fin-row-0");
  await app.press("text:TVA");
  await app.waitFor("id:fin-vat-due");
  await app.press("text:Sessions de caisse");
  await app.waitFor("id:fin-shifts");
  await app.waitFor("text:Ouverte");
});

test("a sealed backup is made, checked and restored", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-settings");
  await app.press("id:settings-sync");
  await app.waitFor("text:Non configurées");
  await app.type("id:bk-pass", "correct horse battery");
  await app.type("id:bk-pass2", "correct horse battery");
  await app.clear("id:bk-dir");
  await app.type("id:bk-dir", `/tmp/nb-ui-backups-${Date.now()}`);
  await app.press("id:bk-save");
  await app.waitFor("text:Réglages des sauvegardes enregistrés.");
  await app.waitFor("id:bk-now");
  await app.press("id:bk-now");
  await seen(/^Sauvegarde faite : nsh-business-\d{8}-\d{6}\.nbk$/);
  await app.waitFor("id:bk-file-0");
  await seen(/^Dernière sauvegarde : /);
  await app.press("id:bk-check-0");
  await seen(/ s'ouvre et est saine\.$/);
  await app.press("id:bk-restore-0");
  await app.waitFor("id:bk-restore-confirm");
  await app.press("id:bk-restore-confirm");
  await app.waitFor("text:Données restaurées.");
  await app.waitFor("text:Ventes brutes");
});

test("customers are imported with their balances after a check", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("text:Clients et crédit");
  await app.press("id:nav-customers");
  await app.waitFor("id:customers-import");
  await app.press("id:customers-import");
  await app.waitFor("id:import-text");
  await app.paste("id:import-text", { text: "Nom;Téléphone;Solde\nAhmed Import;0555;1200\nKarim Import;0666;\n" });
  // The columns are guessed, with samples; the rows show as they will be read.
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "Ahmed Import · Karim Import"));
  await app.waitFor("id:import-row-1");
  expect(app.texts()).toContain("Ahmed Import");
  await app.press("id:import-check");
  await app.waitFor("id:import-ready");
  await app.press("id:import-run");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "2 client(s) importé(s)"));
});

test("a sale's receipt and invoice are previewed, and the printer is set", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.press("id:pay-confirm");
  await seen(/^Vente FV-01-\d\d-A\d+ enregistrée/);
  // The sales at hand at the till (F1): reprint the last one.
  await app.press("id:pos-sales");
  await app.waitFor("id:pos-sale-print-0");
  await app.press("id:pos-sale-print-0");
  // No printer yet: a PDF only.
  await seen(/^Imprimer la vente FV-01-/);
  await app.waitFor("text:Aucune imprimante de tickets : vous pouvez enregistrer un PDF.");
  await app.waitFor("id:print-preview");
  await app.waitFor("id:print-pdf");
  expect(app.texts()).not.toContain("Imprimer");
  await app.press("text:Facture");
  await app.waitFor("id:print-sheet");
  await app.waitFor("id:print-preview");
  await app.press("id:dialog-close");
  // A network printer, added and tested.
  await app.press("id:nav-settings");
  await app.press("id:settings-printing");
  await app.waitFor("text:Aucune imprimante");
  await app.waitFor("id:doc-sample");
  await app.press("id:printer-add");
  await app.waitFor("id:printer-name");
  await app.type("id:printer-name", "Caisse");
  await app.type("id:printer-addr", "127.0.0.1:1");
  await app.press("id:printer-save");
  await app.waitFor("id:printer-row-0");
  await app.waitFor("text:Réseau 127.0.0.1:1 · Rouleau 80 mm");
  await app.press("id:printer-test-0");
  await app.waitFor("text:Impression impossible. Vérifiez que l'imprimante est allumée et son adresse.");
  // A label printer on a 40 × 30 mm roll of labels.
  await app.press("id:printer-add");
  await app.type("id:printer-name", "Zebra");
  await app.press("text:USB / Bluetooth / partagée");
  await app.type("id:printer-path", "/tmp/nb-no-such-printer");
  await app.press("text:Rouleau d'étiquettes");
  await app.waitFor("id:printer-label-w");
  await app.press("id:printer-save");
  await app.waitFor("text:Port /tmp/nb-no-such-printer · Étiquettes 40 × 30 mm");
  // What receipts show: the notice, saved for the shop.
  await app.type("id:doc-notice", "Toute réclamation sous 24 heures");
  await app.press("id:doc-save");
  await app.waitFor("text:Réglages des documents enregistrés.");
});

test("bon par bon: the old debt is paid with a sale, and a receipt is corrected at the till", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const seen = (re: RegExp) => app.waitUntil(() => app.texts().some((t) => re.test(plain(t))));
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  // A first sale leaves something owing.
  await app.press("id:pos-item-0");
  await app.press("id:pos-customer");
  await app.type("id:cust-search", "Boulangerie");
  await app.waitFor("id:cust-pick-0");
  await app.press("id:cust-pick-0");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-balance");
  await app.clear("id:pay-given");
  await app.type("id:pay-given", "100");
  await app.press("id:pay-confirm");
  await seen(/^Vente FV-01-\d\d-A\d+ enregistrée/);
  // The next one: the customer pays it and all they owe.
  await app.press("id:pos-item-0");
  await app.press("id:pos-customer");
  await app.type("id:cust-search", "Boulangerie");
  await app.waitFor("id:cust-pick-0");
  await app.press("id:cust-pick-0");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-balance");
  await app.waitFor("text:Ancien solde");
  await app.press("id:pay-quick-1");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "0,00 DA"));
  await app.press("id:pay-confirm");
  await seen(/^Vente FV-01-\d\d-A\d+ enregistrée/);
  // The sales at hand: the last one reopened, one more item, paid.
  await app.press("id:pos-sales");
  await app.waitFor("id:pos-sale-correct-0");
  await app.press("id:pos-sale-correct-0");
  await app.waitFor("id:pos-correcting");
  await seen(/^Correction du bon FV-01-/);
  await app.press("id:pos-item-1");
  await app.press("id:pos-pay");
  await app.waitFor("id:pay-confirm");
  await app.press("id:pay-confirm");
  await seen(/^Vente FV-01-\d\d-A\d+ enregistrée/);
  await app.press("id:pos-sales");
  await seen(/^Corrigée → FV-01-/);
});

test("an invoice template is copied, edited live and used for printing", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-settings");
  await app.press("id:settings-printing");
  await app.waitFor("id:tpl-row-0");
  await app.waitFor("text:Classique");
  await app.waitFor("id:tpl-preview");
  // Invoices: the modern layout is in use; copy it.
  await app.press("text:Facture");
  await app.waitFor("text:Moderne");
  await app.press("id:tpl-copy-0");
  await app.waitFor("id:tpl-name");
  await app.waitFor("id:tpl-preview");
  await app.press("id:tpl-accent-2");
  await app.press("text:Lignes alternées");
  // A text with a variable, at the end.
  await app.press("id:tpl-add");
  await app.waitFor("id:tpl-section-editor");
  await app.paste("id:tpl-text", { text: "RIB 0020 " });
  await app.press("id:tpl-var-shop.name");
  await app.waitFor("text:RIB 0020 {shop.name}");
  await app.press("id:tpl-bold");
  await app.press("id:tpl-save");
  await app.waitFor("text:Modèles enregistrés");
  // The shop's template is the one invoices print with now.
  await app.waitFor("text:Moderne (copie)");
  await app.waitFor("id:tpl-delete-2");
  await app.waitUntil(() => app.texts().filter((t) => t === "Utilisé").length === 1);
  // Removed: back to the built-in layout.
  await app.press("id:tpl-delete-2");
  await app.waitUntil(() => !app.texts().includes("Moderne (copie)"));
});

test("the till is arranged for scanning: quantity in one press, pay and print", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  // Arranged: the cart as a table.
  await app.press("id:pos-customize");
  await app.press("id:pos-layout-use-1");
  await app.press("id:pos-customize-done");
  await app.type("id:pos-search", "Bimo");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.waitFor("id:cart-line-0");
  // One press on the quantity: the number alone.
  await app.press("id:cart-qty-0");
  await app.waitFor("id:quick-value");
  await app.press("id:quick-3");
  await app.press("id:quick-apply");
  await app.waitUntil(() => !app.shows("id:quick-value"));
  await app.waitUntil(() => app.texts().some((t) => plain(t).startsWith("3 ")));
  // The keys behind the help icon.
  await app.press("id:pos-keys");
  await app.waitFor("text:Raccourcis clavier");
  await app.press("text:Fermer");
  // Pay and print: the receipt dialog follows the sale (no printer: a PDF).
  await app.press("id:pos-pay-print");
  await app.waitFor("id:pay-confirm");
  await app.press("id:pay-confirm");
  await app.waitFor("id:print-preview");
  await app.press("id:dialog-close");
  // The screen edited in place: a column wider, a panel lower, kept.
  await app.press("id:pos-layout-edit");
  await app.waitFor("id:pos-edit-bar");
  await app.press("id:pos-col-wide-0");
  await app.press("id:pos-down-1-0");
  await app.press("id:pos-edit-save");
  await app.waitFor("text:Disposition enregistrée");
  await app.waitUntil(() => !app.shows("id:pos-edit-bar"));
  // Back to the shop's layout for the other tests on this device.
  await app.press("id:pos-customize");
  await app.press("id:pos-layout-use-0");
  await app.press("id:pos-customize-done");
});

test("the sidebar narrows to icons, and the debts and expenses reports open", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("id:toggle-sidebar");
  await app.waitUntil(() => !app.texts().includes("Catalogue"));
  await app.press("id:toggle-sidebar");
  await app.waitFor("text:Catalogue");
  await app.press("text:Rapports");
  await app.press("id:nav-party_reports");
  await app.waitFor("text:Clients débiteurs");
  await app.press("text:Dû aux fournisseurs");
  await app.waitFor("text:Fournisseurs créanciers");
  await app.press("text:Dépenses");
  await app.waitFor("id:party-tabs");
});

test("the bell, help and the user's menu; a sale paid at once; a panel set larger with a bottom bar", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  // What wants attention: products out of stock in the demo shop.
  await app.waitFor("id:notifications-count");
  await app.press("id:open-notifications");
  await app.waitFor("id:notifications");
  await app.waitUntil(() => app.texts().some((t) => plain(t).includes("en rupture")));
  await app.press("id:alert-0");
  await app.waitFor("text:Produits");
  // Help: the keyboard's shortcuts.
  await app.press("id:open-help");
  await app.press("text:Raccourcis clavier");
  await app.waitFor("text:Partout");
  await app.press("id:dialog-close");
  // The person's own menu.
  await app.press("id:open-user");
  await app.waitFor("text:Personnaliser l'apparence");
  await app.press("text:Personnaliser l'apparence");
  await app.waitFor("text:Couleur d'accent");
  // Paid at once: cash, the exact amount, no payment screen.
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-item-0");
  await app.waitFor("id:cart-line-0");
  await app.press("id:pos-pay-now");
  await app.waitUntil(() => app.texts().some((t) => plain(t).startsWith("Vente ")));
  await app.waitUntil(() => !app.shows("id:cart-line-0"));
  expect(app.shows("id:pay-confirm")).toBe(false);
  // A panel's own options: the search larger; a bar along the bottom.
  await app.press("id:pos-layout-edit");
  await app.waitFor("id:pos-edit-bar");
  await app.press("id:pos-opts-0-0");
  await app.press("text:Taille du texte");
  await app.press("text:Très grande");
  await app.waitUntil(() => !app.texts().includes("Taille du texte"));
  await app.press("id:pos-add-strip");
  await app.press("text:Bande en bas");
  await app.press("text:Barre du panier");
  await app.waitFor("id:pos-strip-bottom-0");
  await app.press("id:pos-edit-save");
  await app.waitFor("id:pos-cart-bar");
  await app.waitFor("id:bar-total");
  // Back to the shop's layout for the other tests on this device.
  await app.press("id:pos-customize");
  await app.press("id:pos-layout-use-0");
  await app.press("id:pos-customize-done");
});

test("the dashboard is arranged; favourites are marked and found; carts on hold as badges", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  // Arrange: hide a card, add another, back to the default.
  await app.press("id:home-arrange");
  await app.waitFor("id:home-edit-bar");
  await app.press("id:home-opts-revenue");
  await app.press("text:Masquer");
  await app.press("id:home-add");
  await app.press("text:Plus gros débiteurs");
  await app.waitFor("id:home-card-debts");
  await app.press("id:home-arrange");
  await app.waitUntil(() => !app.shows("id:home-edit-bar"));
  expect(app.texts()).not.toContain("Ventes brutes");
  await app.press("id:home-arrange");
  await app.press("id:home-reset");
  await app.press("id:home-arrange");
  await app.waitFor("text:Ventes brutes");
  // A favourite from the products' list, then the list of favourites.
  await app.press("text:Catalogue");
  await app.press("id:nav-products");
  await app.waitFor("id:product-more >> nth=0");
  await app.press("id:product-more >> nth=0");
  await app.press("text:Ajouter aux favoris");
  await app.waitFor("text:Ajouté aux favoris");
  await app.press("id:products-favorites");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–1 sur 1"));
  // The carts on hold as badges: take one back with a press.
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-item-0");
  await app.press("id:pos-layout-edit");
  await app.waitFor("id:pos-edit-bar");
  await app.press("id:pos-add-strip");
  await app.press("text:Bande en haut");
  await app.press("text:Paniers en attente");
  await app.press("id:pos-edit-save");
  await app.waitFor("id:pos-held-bar");
  await app.press("id:pos-item-0");
  await app.press("id:held-bar-hold");
  await app.waitFor("id:held-badge-0");
  await app.waitUntil(() => !app.shows("id:cart-line-0"));
  await app.press("id:held-badge-0");
  await app.waitFor("id:cart-line-0");
  await app.waitUntil(() => !app.shows("id:held-badge-0"));
  await app.press("id:pos-customize");
  await app.press("id:pos-layout-use-0");
  await app.press("id:pos-customize-done");
});

test("more filters, labels on an A4 sheet, held stock and an export on every list", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  const shows = (s: string) => () => app.texts().some((t) => plain(t) === s);
  await app.waitFor("text:Ventes brutes");
  // The stock: only what is held for orders (none in the demo).
  await app.press("text:Stock");
  await app.press("id:nav-inventory");
  await app.waitUntil(shows("1–25 sur 2 000"));
  expect(app.shows("id:stock-export")).toBe(true);
  await app.press("id:stock-reserved");
  await app.waitFor("text:Aucun article");
  await app.press("id:tab-movements");
  await app.waitFor("id:movements-export");
  await app.press("text:Catalogue");
  await app.waitFor("id:nav-products");
  await app.press("id:nav-products");
  await app.waitUntil(shows("1–25 sur 2 000"));
  expect(app.shows("id:products-export")).toBe(true);
  // A filter from the menu: services only (the demo has none), then cleared.
  await app.press("id:products-more-filters");
  await app.press("text:Type");
  await app.press("text:Service");
  await app.waitFor("text:Aucun résultat");
  await app.press("text:Effacer les filtres >> nth=0");
  await app.waitUntil(shows("1–25 sur 2 000"));
  // Labels on an A4 sheet of 21.
  await app.press("id:product-more >> nth=0");
  await app.press("text:Imprimer l'étiquette");
  await app.waitFor("id:label-size");
  await app.press("id:label-size");
  await app.press("text:A4 · 3 × 7 — 63.5 × 38.1 mm");
  await app.waitFor("id:label-skip");
  await app.waitUntil(() => app.texts().some((t) => plain(t).startsWith("21 étiquettes par feuille")));
  await app.press("id:dialog-close");
});

test("the till's head: quick access to the pages, full screen without the app's bar", async () => {
  const app = await open(".", { locale: "fr-DZ", width: 1360, height: 860, platform: "linux" });
  await app.waitFor("text:Ventes brutes");
  await app.press("id:nav-sell");
  await app.waitFor("id:till-open");
  await app.type("id:till-float", "0");
  await app.press("id:till-open");
  await app.waitFor("id:pos-quick");
  expect(app.shows("id:open-palette")).toBe(true);
  // Full screen: no sidebar, no bar at the top; the quick menu still goes.
  await app.press("id:pos-fullscreen");
  await app.waitUntil(() => !app.shows("id:open-palette"));
  await app.press("id:pos-quick");
  await app.press("text:Accueil");
  await app.waitFor("text:Ventes brutes");
  await app.waitFor("id:open-palette");
});
