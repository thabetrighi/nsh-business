# NSH Business

Universal, local-first business management platform: POS, inventory, sales, purchasing, debts & installments, full double-entry accounting and distribution. Built with Rust and [Native Shell (NSH)](https://nsh.dev) for Windows, macOS, Linux, Android, iOS and Web.

منصة شاملة لإدارة الأعمال تعمل دون إنترنت، مبنية بـ Rust وNSH لجميع المنصات.

## الحالة
المرحلة **M0** (التحقق من الإطار). التفاصيل في [`docs/PROGRESS.md`](docs/PROGRESS.md).

## الوثائق
| الملف | المحتوى |
|---|---|
| [`docs/PRD.md`](docs/PRD.md) | وثيقة متطلبات المنتج |
| [`docs/PLAN.md`](docs/PLAN.md) | خطة التنفيذ (المراحل M0–M18) |
| [`docs/PROGRESS.md`](docs/PROGRESS.md) | حالة كل مهمة |
| [`docs/NEEDS_OWNER.md`](docs/NEEDS_OWNER.md) | ما ينتظر قرار أو اختبار المالك |
| [`docs/spikes/`](docs/spikes/) | تقارير التحقق التقني |
| [`docs/adr/`](docs/adr/) | سجل القرارات المعمارية |
| [`CLAUDE.md`](CLAUDE.md) | قواعد العمل والشيفرة |

## المتطلبات (Linux)
- Rust ≥ 1.85، NSH CLI 0.4.0، GTK ≥ 4.14 (`libgtk-4-dev`)، clang.

```bash
cd spikes/m0-hello && cargo run      # نموذج Spike
nsh test spikes/m0-hello             # اختبار الواجهة headless
```

© جميع الحقوق محفوظة.
