# CLAUDE.md — قواعد العمل الثابتة لمشروع NSH Business

اقرأ هذا الملف في بداية كل جلسة. المستودع هو المرجع الوحيد؛ لا تعتمد على ذاكرة جلسات سابقة.

## بداية كل جلسة
1. اقرأ `docs/PROGRESS.md` لمعرفة المهمة التالية.
2. اقرأ `docs/NEEDS_OWNER.md` وأي ADR جديد في `docs/adr/`.
3. ارجع إلى `docs/PLAN.md` (المهام ومعايير القبول) و`docs/PRD.md` (المتطلبات) للمهمة المختارة فقط.

## نهاية كل جلسة
1. `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test` ناجحة.
2. حدّث `docs/PROGRESS.md` (الحالة + ملاحظة قصيرة + التاريخ).
3. أضف ما يحتاج تدخل المالك إلى `docs/NEEDS_OWNER.md`.
4. Commit برسالة تبدأ برقم المهمة: `M1.3: Money type with allocation`.

## قواعد الشيفرة (غير قابلة للتنازل)
- **المال:** لا `f32`/`f64` أبداً في المال أو الكميات. المال عدد صحيح بأصغر وحدة (`i64` minor units) + عملة. الكميات `Decimal`.
- **المخزون:** حركات append-only فقط. الرصيد إسقاط قابل لإعادة البناء.
- **المحاسبة:** القيد المرحّل لا يُعدَّل ولا يُحذف؛ التصحيح بقيد عكسي. كل قيد متوازن.
- **الذرية:** كل حالة استخدام تكتب بيانات مالية/مخزنية تعمل في معاملة واحدة تشمل Outbox وAudit.
- **Idempotency:** كل عملية لها `operation_id` (UUIDv7) فريد؛ التكرار يعيد النتيجة نفسها.
- **الحدود:** `nb-domain` بلا IO وبلا اعتماد على NSH. الواجهة لا تحتوي قواعد أعمال.
- **النصوص:** كل نص ظاهر عبر `nb-i18n` (ar/en/fr). لا نصوص مضمّنة.
- **RTL:** استخدم `LayoutDirection` وخصائص منطقية؛ لا افتراض لاتجاه يسار/يمين.
- **الأخطاء:** لا `unwrap()`/`expect()` في شيفرة الإنتاج إلا مع تعليق يبرر الاستحالة.
- **الترحيلات:** أي تغيير مخطط = ملف ترحيل جديد + اختبار. لا تعديل لترحيل منشور.

## دروس مستفادة (إلزامية)
- **الدفع مشروط بالفحوص:** `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && git commit … && git push` — بـ `&&` لا بـ `;`.
- **كل تعديل نصي آلي على الشيفرة يتحقق من التطابق** (`assert old in s`)؛ `cargo fmt` يعيد التنسيق فيفشل الاستبدال بصمت.
- **المهام والمؤقتات في NSH تنتهي مع مالكها:** مؤقت يُنشأ داخل شاشة أو نافذة يُلغى عند إزالتها. ما يجب أن يستمر يُنشأ من طبقة دائمة (الجذر).
- **لا يُحكم على ترتيب نص ثنائي الاتجاه من لقطة بالعين:** يُقارن بسطر مرجعي يسار-يمين.
- **جدول `nsh-base` يسار-يمين فقط:** في العربية تُعكس الأعمدة يدوياً ويُثبَّت الجدول LTR (انظر `screens/products.rs`).
- **الخطوط على سطح المكتب:** `Root` الذي يُشغَّل مباشرة لا يسجّل خطوط `assets!()`؛ تُمرَّر بـ `Root::font` في `main.rs` (وإلا يُستعمل خط النظام بصمت). Cairo SemiBold مولّد عند wght 650 لأن 600 في Cairo يكاد لا يختلف عن العادي.
- **قاعدة البيانات لكل نسخة تطبيق:** `AppState::db()` يبدأ خيطاً خاصاً (اختبارات الواجهة تعمل في عملية واحدة فتتشارك أي حالة عامة). سطح المكتب يحدد الملف في `main.rs` (`NB_DATA_DIR` أو مجلد بيانات النظام)؛ دونه قاعدة في الذاكرة مع بيانات العرض.
- **اللقطات:** شغّل التطبيق مع `NB_DATA_DIR` جديد في كل لقطة، واحذف `~/.local/share/default/storage.json`.
- **إعدادات المظهر تعيد بناء الشجرة:** الحالة الدائمة (المسار، البيانات) تبقى في `AppState` خارج الجذر المعاد بناؤه.
- **مساحة القرص في بيئة Claude محدودة (~43 GB):** `nsh test` يبني في `apps/client/target` منفصلاً عن `target` الجذر. شغّل أوامر cargo بـ `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=apps/client/target` لتشارك المجلد نفسه، ولا تُبقِ `target` الجذر. عند الامتلاء: `find apps/client/target/debug/deps -maxdepth 1 -type f -mmin +45 -size +20M -delete` (نسخ قديمة بتجزئات أخرى).
- **الصلاحيات في حالات الاستخدام لا في الواجهة:** كل حالة استخدام كاتبة تستدعي `w.require(Permission, Target)`؛ الواجهة تخفي فقط (`st.can`, `st.shows`).
- **نوافذ الحوار:** `dialog_frame` يتقلص ويمرّر جسمه؛ القوائم التي تُحمَّل بعد فتح النموذج تُمرَّر كـ `Memo` لا كقيمة.
- **اختبارات الواجهة:** `app.type` ينهي السطر بزر الإدخال؛ لنص متعدد الأسطر استعمل `app.paste(sel, { text })`. الأرقام في النصوص معزولة bidi: قارن عبر `plain()`.
- **أرقام داخل نص عربي:** الإشارة داخل منطقة العزل مع الرقم (`Format::money_signed`)؛ فاصل الآلاف U+00A0 (الخط لا يحوي U+202F).

## الأدوات والأوامر
- NSH: الإصدار المثبت `0.4.0` (لا ترقية دون ADR).
- تثبيت NSH في بيئة Claude: nsh.dev محجوب من الشل؛ نزّل `https://github.com/longbridge/nsh/releases/download/v0.4.0/nsh-linux-x86_64.tar.gz` وفكّه في `~/.nsh` ثم اربط `~/.nsh/nsh/bin/nsh`.
- متطلبات Linux: `apt-get install libgtk-4-dev clang xvfb imagemagick` (GTK ≥ 4.14).
- متغيرات تطوير للتطبيق: `NB_LANG=ar|fr|en`، `NB_APPEARANCE=light|dark|system`، `NB_WINDOW=1360x860`، `NB_ROUTE=products|settings|design|customers`، `NB_OPEN=add_product|palette`، `NB_ACCENT=blurple|ocean|emerald|teal|amber|rose|slate`، `NB_FONT=cairo|tajawal|almarai|readex|plex`، `NB_TEXT=s|m|l|xl`، `NB_DENSITY=compact|comfortable|spacious`. الإعدادات تُحفظ في `~/.local/share/default/storage.json` (احذفه بين اللقطات).
- تشغيل واجهة دون شاشة: `Xvfb :99 &` ثم `DISPLAY=:99 GDK_BACKEND=x11 <binary>` ولقطة بـ `import -window root`.
- اختبارات الواجهة: `nsh test <app-dir>`.
- الويب يحتاج `rustup target add wasm32-unknown-unknown` + `wasm-bindgen-cli`؛ غير متاح في بيئة Claude (انظر NEEDS_OWNER) — يُبنى عبر GitHub Actions.

## مراجع NSH
- فهرس الوثائق: https://nsh.dev/llms.txt (صفحات `.md` قابلة للقراءة بـ WebFetch).
- مصدر الحزم المحلي: `~/.cargo/registry/src/*/nsh-core-0.4.0` (المرجع الأدق عند غموض الوثائق).

## اللغة
- **كل الردود والرسائل للمالك بالعربية دائماً** (طلب صريح من المالك)، وكذلك الوثائق والتقارير. الشيفرة والتعليقات والمعرّفات بالإنجليزية.
