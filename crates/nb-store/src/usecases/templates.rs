//! Document templates (M12.11): the shop's own layouts next to the
//! built-in ones, and which one each kind of document prints with.
//! A template is checked before it is kept (`Template::check`); one that
//! no longer reads (a newer app wrote it, the row was damaged) is never
//! used — the document prints with the built-in layout instead, so a
//! template can never stop the shop from printing.

use nb_domain::RuleError;
use nb_domain::access::{Permission, Target};
use nb_domain::template::{DocKind, Template, builtin, usable};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::uow::OpMeta;
use crate::usecases::sales::the_org;
use crate::{Error, Result, Store};

/// The prefix of a built-in template's id.
pub const BUILTIN: &str = "builtin:";

/// A template as the settings list it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateRow {
    /// `builtin:<code>` or the shop's template id.
    pub id: String,
    /// The shop's name for it; a built-in one's code (the app names it).
    pub name: String,
    pub builtin: bool,
    /// The one this kind of document prints with.
    pub chosen: bool,
    pub template: Template,
}

/// A template to keep: a new one (`id` empty) or a change to the shop's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub template: Template,
}

fn choice(c: &Connection, org: &str, kind: DocKind) -> Result<Option<String>> {
    Ok(c.query_row(
        "SELECT choice FROM document_template_choice WHERE org_id = ?1 AND kind = ?2",
        params![org, kind.code()],
        |r| r.get(0),
    )
    .optional()?)
}

fn own(c: &Connection, org: &str, kind: DocKind) -> Result<Vec<(String, String, String)>> {
    let mut s = c.prepare(
        "SELECT id, name, body FROM document_templates
         WHERE org_id = ?1 AND kind = ?2 AND deleted_at IS NULL ORDER BY created_at",
    )?;
    let rows = s
        .query_map(params![org, kind.code()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// The templates of a kind: the built-in ones, then the shop's (those
/// that still read).
pub fn list_templates(store: &Store, kind: DocKind) -> Result<Vec<TemplateRow>> {
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let org = org.to_string();
        let chosen = choice(r.conn(), &org, kind)?;
        let mut rows: Vec<TemplateRow> = builtin(kind)
            .into_iter()
            .map(|(code, template)| TemplateRow {
                id: format!("{BUILTIN}{code}"),
                name: code.to_owned(),
                builtin: true,
                chosen: false,
                template,
            })
            .collect();
        for (id, name, body) in own(r.conn(), &org, kind)? {
            if let Some(template) = Template::parse(&body).filter(|t| t.kind == kind) {
                rows.push(TemplateRow {
                    id,
                    name,
                    builtin: false,
                    chosen: false,
                    template,
                });
            }
        }
        let at = chosen
            .and_then(|c| rows.iter().position(|row| row.id == c))
            .unwrap_or(0);
        if let Some(row) = rows.get_mut(at) {
            row.chosen = true;
        }
        Ok(rows)
    })
}

/// What a kind of document prints with: the chosen template, or the first
/// built-in one when none is chosen or the chosen one does not read.
pub fn template_for(store: &Store, kind: DocKind) -> Result<Template> {
    store.read(|r| {
        let Ok((org, _)) = the_org(r.conn()) else {
            return Ok(usable(kind, None));
        };
        let org = org.to_string();
        let Some(c) = choice(r.conn(), &org, kind)? else {
            return Ok(usable(kind, None));
        };
        if let Some(code) = c.strip_prefix(BUILTIN) {
            return Ok(builtin(kind)
                .into_iter()
                .find(|(k, _)| *k == code)
                .map(|(_, t)| t)
                .unwrap_or_else(|| usable(kind, None)));
        }
        let body: Option<String> = r
            .conn()
            .query_row(
                "SELECT body FROM document_templates WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
                params![c, org],
                |r| r.get(0),
            )
            .optional()?;
        Ok(usable(kind, body.as_deref()))
    })
}

fn name_of(input: &TemplateInput) -> std::result::Result<String, Vec<RuleError>> {
    let name = input.name.trim().to_owned();
    let mut e = match input.template.check() {
        Ok(()) => Vec::new(),
        Err(e) => e,
    };
    if name.is_empty() {
        e.push(RuleError::on("name", "required"));
    } else if name.chars().count() > 60 {
        e.push(RuleError::on("name", "template.name_long"));
    }
    if e.is_empty() { Ok(name) } else { Err(e) }
}

/// Keeps a template (checked first). Returns its id.
pub fn save_template(store: &mut Store, meta: OpMeta, input: TemplateInput) -> Result<String> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::TemplatesSaveTemplate {
            input: input.clone(),
        },
    )?;
    let name = name_of(&input)?;
    store.write(meta, |w| {
        let org = w.org()?.to_string();
        w.require(Permission::SettingsEdit, &Target::org())?;
        let kind = input.template.kind;
        let body = input.template.to_json();
        let id = match input.id.as_deref().filter(|i| !i.is_empty()) {
            Some(id) => {
                let was: Option<String> = w
                    .tx()
                    .query_row(
                        "SELECT kind FROM document_templates WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
                        params![id, org],
                        |r| r.get(0),
                    )
                    .optional()?;
                match was {
                    None => return Err(Error::NotFound("template")),
                    Some(k) if k != kind.code() => {
                        return Err(RuleError::on("kind", "template.kind").into());
                    }
                    Some(_) => {}
                }
                w.tx().execute(
                    "UPDATE document_templates SET name = ?2, body = ?3, updated_at = ?4, version = version + 1
                     WHERE id = ?1",
                    params![id, name, body, w.now()],
                )?;
                id.to_owned()
            }
            None => {
                let id = nb_core::id::idgen::uuid().to_string();
                w.tx().execute(
                    "INSERT INTO document_templates (id, org_id, kind, name, body, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                    params![id, org, kind.code(), name, body, w.now()],
                )?;
                id
            }
        };
        let what = serde_json::json!({ "kind": kind.code(), "name": name });
        w.event("documents.template_saved", "document_template", &id, &what)?;
        w.audit("save_template", "document_template", &id, &what)?;
        Ok(id)
    })
}

/// Removes one of the shop's templates; if documents printed with it,
/// they go back to the built-in layout.
pub fn delete_template(store: &mut Store, meta: OpMeta, id: &str) -> Result<()> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::TemplatesDeleteTemplate { id: id.to_owned() },
    )?;
    store.write(meta, |w| {
        let org = w.org()?.to_string();
        w.require(Permission::SettingsEdit, &Target::org())?;
        let n = w.tx().execute(
            "UPDATE document_templates SET deleted_at = ?3, version = version + 1
             WHERE id = ?1 AND org_id = ?2 AND deleted_at IS NULL",
            params![id, org, w.now()],
        )?;
        if n == 0 {
            return Err(Error::NotFound("template"));
        }
        w.tx().execute(
            "DELETE FROM document_template_choice WHERE org_id = ?1 AND choice = ?2",
            params![org, id],
        )?;
        w.event("documents.template_deleted", "document_template", id, &id)?;
        w.audit("delete_template", "document_template", id, &id)?;
        Ok(())
    })
}

/// Which template a kind of document prints with (`builtin:<code>` or one
/// of the shop's).
pub fn choose_template(store: &mut Store, meta: OpMeta, kind: DocKind, id: &str) -> Result<()> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::TemplatesChooseTemplate {
            kind: kind.clone(),
            id: id.to_owned(),
        },
    )?;
    store.write(meta, |w| {
        let org = w.org()?.to_string();
        w.require(Permission::SettingsEdit, &Target::org())?;
        let known = match id.strip_prefix(BUILTIN) {
            Some(code) => builtin(kind).iter().any(|(k, _)| *k == code),
            None => w
                .tx()
                .query_row(
                    "SELECT 1 FROM document_templates
                     WHERE id = ?1 AND org_id = ?2 AND kind = ?3 AND deleted_at IS NULL",
                    params![id, org, kind.code()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some(),
        };
        if !known {
            return Err(Error::NotFound("template"));
        }
        w.tx().execute(
            "INSERT INTO document_template_choice (org_id, kind, choice, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (org_id, kind) DO UPDATE SET choice = ?3, updated_at = ?4",
            params![org, kind.code(), id, w.now()],
        )?;
        let what = serde_json::json!({ "kind": kind.code(), "template": id });
        w.event("documents.template_chosen", "organization", &org, &what)?;
        w.audit("choose_template", "organization", &org, &what)?;
        Ok(())
    })
}
