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
