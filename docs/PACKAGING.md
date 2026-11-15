# التغليف والنشر (M14.7)

كيف يُبنى التطبيق لكل منصة، وأين يُنشر، وما يحتاجه كل منها. الأداة واحدة: `nsh package --target <المنصة>` من `apps/client`، وتقرأ من `nsh.json` الاسم والمعرّف والإصدار والأيقونة (`packaging/icon.png`) ومصدر التحديثات والصلاحيات.

## الحالة
| المنصة | ما يُبنى | أين | آلياً؟ |
|---|---|---|---|
| **Windows 10/11** | مثبّت `NSH-Business-Setup-<الإصدار>.exe`، وملف مضغوط | GitHub Releases: `v0.1.N` (الأحدث، مع `latest.json` للتحديث التلقائي) و`windows-latest` | ✅ مع كل تعديل (`client-windows.yml`) |
| **Android 8+** | `nsh-business.apk` (arm64 + armv7) | `android-latest` | ✅ مع كل تعديل (`client-android.yml`). **يُوقَّع بمفتاح التطوير إلى أن يضع المالك مفتاح الإصدار (O-39)** |
| **الويب** | موقع ثابت | ملف البناء في `client-web.yml` | ✅ |
| **Linux** | مجلد ومضغوط `.tar.gz` مع ملف `.desktop` والأيقونة | يُبنى يدوياً (الأمر أدناه) | يدوي |
| **macOS** | `.app` | يحتاج جهاز Mac | يدوي |
| **iOS** | `.ipa` | يحتاج Mac وحساب مطوّر Apple | يدوي |

## Windows
- **المثبّت يصنعه `nsh package` نفسه** إذا وجد Inno Setup 6 (يثبّته الـ CI بـ `choco install innosetup`):
  - **لكل مستخدم، دون صلاحيات المدير.** يثبّت في `%LOCALAPPDATA%\Programs`، فيستطيع التطبيق أن يحدّث نفسه هناك (ADR-0011).
  - اختصار في قائمة ابدأ، وظهور في «التطبيقات المثبتة»، والتثبيت فوق نسخة سابقة.
- **البيانات** في `%APPDATA%\dz.nshbusiness.app\store.db`. لا يمسها إلغاء التثبيت ولا التحديث.
- **الإصدار:** `0.1.<رقم تشغيل البناء>`، يكتبه الـ CI في `nsh.json` قبل الحزم.
- **غير موقّع رقمياً:** يظهر تحذير SmartScreen («مزيد من المعلومات» ← «تشغيل على أي حال»). **إزالته تحتاج شهادة توقيع برمجيات** (Code Signing، يشتريها المالك).

## Android
- **الإصدار** `0.1.<run>` **ورقم البناء** `<run>`. يرتفعان دائماً، فيقبل الهاتف التحديث.
- **التوقيع بمفتاح الإصدار:** ضرورة قبل التوزيع على أي هاتف حقيقي. دونه يوقّع كل بناء بمفتاح مختلف، فيرفض الهاتف تثبيت النسخة الجديدة فوق القديمة. الخطوات (مرة واحدة، على حاسوب المالك):
  ```
  keytool -genkeypair -v -keystore nsh-business.keystore -alias nshbusiness -keyalg RSA -keysize 4096 -validity 10000
  base64 -w0 nsh-business.keystore   # على Windows: certutil -encode
  ```
  ثم في GitHub ← Settings ← Secrets and variables ← Actions، ثلاثة أسرار:
  - `ANDROID_KEYSTORE_BASE64` (الناتج)؛
  - `ANDROID_KEY_ALIAS` (`nshbusiness`)؛
  - `ANDROID_KEYSTORE_PASSWORD`.

  **احفظ الملف وكلمة مروره في مكانين آمنين:** ضياعهما يعني ألا يُحدَّث التطبيق المثبّت أبداً.
- **Google Play:** `nsh package --target android --format aab` بالمفتاح نفسه.

## Linux
```
cd apps/client
CARGO_TARGET_DIR=$PWD/target CARGO_INCREMENTAL=0 nsh package --target linux -o out
```
- **الناتج:**
  - مجلد `NSH Business/` (البرنامج `NSHBusiness` و`nsh/` و`dz.nshbusiness.app.desktop`)؛
  - نسخته المضغوطة؛
  - `NSH-Business-<الإصدار>-linux-x86_64.tar.gz` للتحديث التلقائي.
- **البيانات** في `$XDG_DATA_HOME/dz.nshbusiness.app` (أو `~/.local/share/...`).
- **المتطلبات:** GTK 4.14+ وGStreamer (انظر `CLAUDE.md`).

## macOS وiOS (تحتاج جهاز Mac)
```
nsh package --target macos --arch universal          # .app
nsh package --target ios --sign "Apple Distribution: …" --profile <ملف.mobileprovision>
```
- **macOS:**
  - التوزيع خارج المتجر يحتاج توقيع «Developer ID» وتوثيقاً لدى Apple (notarization)؛
  - البيانات في `~/Library/Application Support/dz.nshbusiness.app`.
- **iOS:** يحتاج حساب Apple Developer (99 دولاراً سنوياً).

## قبل كل إصدار
1. **ملاحظات الإصدار:** حدّث `apps/client/release-notes.md` بالعربية، فهي تظهر للبائع في التطبيق.
2. **الفحوص كلها خضراء:** `ci`، و`audit`، واختبارات الواجهة.
3. **بعد النشر:** نسخة مثبّتة تجد الإصدار من الإعدادات ← حول التطبيق.
