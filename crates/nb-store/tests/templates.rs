//! Document templates (M12.11): the shop keeps its own layouts, chooses
//! one per kind of document, and a broken or removed one never stops
//! printing — the built-in layout takes over.

mod common;

use common::*;
use nb_domain::template::{DocKind, Section, Template, builtin};
use nb_store::Error;
use nb_store::usecases::templates::*;

type R = Result<(), Error>;

fn mine(name: &str) -> TemplateInput {
    let mut t: Template = builtin(DocKind::Invoice).remove(0).1;
    t.style.accent = "#0F766E".into();
    t.sections.push(Section::Text {
        text: "IBAN: DZ00 0000 {shop.name}".into(),
        align: Default::default(),
        size: 0,
        bold: false,
    });
    TemplateInput {
        id: None,
        name: name.into(),
        template: t,
    }
}

#[test]
fn the_shop_keeps_and_chooses_its_templates() -> R {
    let (mut s, setup) = store();
    // Nothing chosen: the first built-in layout.
    let rows = list_templates(&s, DocKind::Invoice)?;
    assert!(rows.iter().all(|r| r.builtin));
    assert!(rows[0].chosen);
    assert_eq!(
        template_for(&s, DocKind::Invoice)?,
        builtin(DocKind::Invoice)[0].1
    );

    let id = save_template(&mut s, meta(&setup, "tpl"), mine("Facture verte"))?;
    choose_template(&mut s, meta(&setup, "tpl"), DocKind::Invoice, &id)?;
    let t = template_for(&s, DocKind::Invoice)?;
    assert_eq!(t.style.accent, "#0F766E");
    let rows = list_templates(&s, DocKind::Invoice)?;
    assert_eq!(rows.iter().filter(|r| r.chosen).count(), 1);
    assert!(
        rows.iter()
            .any(|r| r.id == id && r.chosen && r.name == "Facture verte")
    );
    // Receipts are not touched.
    assert_eq!(
        template_for(&s, DocKind::Receipt)?,
        builtin(DocKind::Receipt)[0].1
    );

    // Changed in place.
    let mut again = mine("Facture verte 2");
    again.id = Some(id.clone());
    again.template.style.size = 11;
    assert_eq!(save_template(&mut s, meta(&setup, "tpl"), again)?, id);
    assert_eq!(template_for(&s, DocKind::Invoice)?.style.size, 11);

    // A built-in one can be chosen too.
    choose_template(
        &mut s,
        meta(&setup, "tpl"),
        DocKind::Invoice,
        "builtin:classic",
    )?;
    assert_eq!(
        template_for(&s, DocKind::Invoice)?,
        builtin(DocKind::Invoice)[1].1
    );
    choose_template(&mut s, meta(&setup, "tpl"), DocKind::Invoice, &id)?;

    // Removed: back to the built-in layout.
    delete_template(&mut s, meta(&setup, "tpl"), &id)?;
    assert_eq!(
        template_for(&s, DocKind::Invoice)?,
        builtin(DocKind::Invoice)[0].1
    );
    assert!(
        list_templates(&s, DocKind::Invoice)?
            .iter()
            .all(|r| r.builtin)
    );
    assert_eq!(
        count(
            &s,
            "SELECT COUNT(*) FROM audit_log WHERE action LIKE '%template%'"
        ),
        6
    );
    Ok(())
}

#[test]
fn a_bad_template_is_refused_and_a_damaged_one_never_prints() -> R {
    let (mut s, setup) = store();
    let mut bad = mine("");
    bad.template.style.accent = "green".into();
    let codes: Vec<_> = match save_template(&mut s, meta(&setup, "tpl"), bad) {
        Err(e) => e.rule_codes().into_iter().map(|(_, c)| c).collect(),
        Ok(_) => panic!("saved a bad template"),
    };
    assert!(codes.contains(&"required"));
    assert!(codes.contains(&"template.color"));
    // An unknown template or one of another kind cannot be chosen.
    assert!(
        choose_template(
            &mut s,
            meta(&setup, "tpl"),
            DocKind::Invoice,
            "builtin:nope"
        )
        .is_err()
    );
    let id = save_template(&mut s, meta(&setup, "tpl"), mine("A"))?;
    assert!(choose_template(&mut s, meta(&setup, "tpl"), DocKind::Receipt, &id).is_err());
    choose_template(&mut s, meta(&setup, "tpl"), DocKind::Invoice, &id)?;
    // The row damaged behind the app's back: the built-in layout prints.
    s.connection()
        .execute(
            "UPDATE document_templates SET body = '{\"version\": 9}'",
            [],
        )
        .map_err(Error::from)?;
    assert_eq!(
        template_for(&s, DocKind::Invoice)?,
        builtin(DocKind::Invoice)[0].1
    );
    assert!(
        list_templates(&s, DocKind::Invoice)?
            .iter()
            .all(|r| r.builtin)
    );
    Ok(())
}
