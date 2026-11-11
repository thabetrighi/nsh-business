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
  await app.press("text:Dépenses");
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
  await app.waitUntil(() =>
    app.texts().some((t) => plain(t) === "Ligne 3 : Saisissez le prix en chiffres, par exemple 120 ou 120,50."),
  );
  await app.clear("id:import-text");
  await app.paste("id:import-text", { text: "Nom;Prix;Catégorie\nCafé moulu;450;Épicerie\nThé vert;220;Épicerie\n" });
  await app.press("id:import-run");
  await app.waitUntil(() => app.texts().some((t) => plain(t) === "1–25 sur 2 002"));
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
  await app.press("text:Stock faible");
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
  await app.press("id:cart-qty-0");
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
});
