//! User-authored skill grouping.
//!
//! Groups are organizational buckets (folders) for skills, independent of the
//! catalog: a skill is assigned to at most one group via `skill_group_members`,
//! keyed by the catalog's stable `skill_id`. The catalog is rebuildable, so
//! `skill_id` is a *weak* reference — leaving the member row in place when a
//! skill is disabled (its row is deleted then recreated with the same id on
//! re-enable) lets the assignment reattach automatically. Dangling rows left by
//! a real delete or a consolidate are migrated (`migrate_members`) or surfaced
//! as missing by the caller's LEFT JOIN against the catalog.

use anyhow::{Context as _, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SkillGroup {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub sort_order: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// A group with its member skill ids and count. Members are returned verbatim;
/// callers LEFT JOIN the catalog to flag any whose `skill_id` is no longer
/// present (deleted/consolidated away) rather than silently dropping them.
#[derive(Clone, Debug, Serialize)]
pub struct GroupWithMembers {
    #[serde(flatten)]
    pub group: SkillGroup,
    pub member_skill_ids: Vec<String>,
    pub member_count: i64,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn new_group_id() -> String {
    format!("group:{}", Uuid::new_v4())
}

/// Trim a text field, mapping empty input to `None` so an empty string clears
/// an optional column instead of storing a blank.
fn optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn list_groups(conn: &Connection) -> Result<Vec<GroupWithMembers>> {
    let mut groups = Vec::new();
    let mut statement = conn
        .prepare(
            "SELECT id, name, description, color, sort_order, created_at_ms, updated_at_ms
             FROM skill_groups ORDER BY sort_order, name",
        )
        .context("Failed to read skill groups")?;
    let group_rows = statement
        .query_map([], row_to_group)
        .context("Failed to read skill groups")?;
    for row in group_rows {
        groups.push(row?);
    }

    let mut members_by_group: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut member_statement = conn
        .prepare(
            "SELECT group_id, skill_id FROM skill_group_members
             ORDER BY group_id, sort_order, skill_id",
        )
        .context("Failed to read skill group members")?;
    let member_rows = member_statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in member_rows {
        let (group_id, skill_id) = row?;
        members_by_group.entry(group_id).or_default().push(skill_id);
    }

    Ok(groups
        .into_iter()
        .map(|group| {
            let members = members_by_group.remove(&group.id).unwrap_or_default();
            let member_count = members.len() as i64;
            GroupWithMembers {
                group,
                member_skill_ids: members,
                member_count,
            }
        })
        .collect())
}

pub fn create_group(
    conn: &mut Connection,
    name: &str,
    description: Option<&str>,
    color: Option<&str>,
    sort_order: i64,
) -> Result<SkillGroup> {
    let tx = conn.transaction().context("Failed to create skill group")?;
    let group = SkillGroup {
        id: new_group_id(),
        name: name.trim().to_string(),
        description: optional_text(description),
        color: optional_text(color),
        sort_order,
        created_at_ms: now_ms(),
        updated_at_ms: now_ms(),
    };
    if group.name.is_empty() {
        anyhow::bail!("Skill group name must not be empty");
    }
    tx.execute(
        "INSERT INTO skill_groups (id, name, description, color, sort_order, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            group.id,
            group.name,
            group.description,
            group.color,
            group.sort_order,
            group.created_at_ms,
            group.updated_at_ms,
        ],
    )
    .context("Failed to insert skill group")?;
    tx.commit().context("Failed to commit skill group")?;
    Ok(group)
}

/// Full replacement of a group's editable fields. `name` must be non-empty;
/// empty `description`/`color` clear those columns.
pub fn update_group(
    conn: &mut Connection,
    id: &str,
    name: &str,
    description: Option<&str>,
    color: Option<&str>,
    sort_order: i64,
) -> Result<SkillGroup> {
    let trimmed_name = name.trim().to_string();
    if trimmed_name.is_empty() {
        anyhow::bail!("Skill group name must not be empty");
    }
    let tx = conn.transaction().context("Failed to update skill group")?;
    let updated = tx
        .execute(
            "UPDATE skill_groups
             SET name = ?2, description = ?3, color = ?4, sort_order = ?5, updated_at_ms = ?6
             WHERE id = ?1",
            params![
                id,
                trimmed_name,
                optional_text(description),
                optional_text(color),
                sort_order,
                now_ms(),
            ],
        )
        .context("Failed to update skill group")?;
    if updated == 0 {
        anyhow::bail!("Unknown skill group: {id}");
    }
    tx.commit().context("Failed to commit skill group update")?;
    get_group(conn, id)
}

pub fn delete_group(conn: &mut Connection, id: &str) -> Result<()> {
    let tx = conn.transaction().context("Failed to delete skill group")?;
    // ON DELETE CASCADE clears members; skills simply return to "ungrouped".
    tx.execute("DELETE FROM skill_groups WHERE id = ?1", params![id])
        .context("Failed to delete skill group")?;
    tx.commit()
        .context("Failed to commit skill group deletion")?;
    Ok(())
}

/// Assign a skill to a group, move it between groups, or remove it. `group_id
/// = None` removes the assignment. The foreign key rejects an unknown group.
pub fn set_skill_group(
    conn: &mut Connection,
    skill_id: &str,
    group_id: Option<&str>,
) -> Result<()> {
    let tx = conn
        .transaction()
        .context("Failed to set skill group assignment")?;
    match group_id {
        Some(group_id) => {
            tx.execute(
                "INSERT INTO skill_group_members (skill_id, group_id, sort_order, added_at_ms)
                 VALUES (?1, ?2, 0, ?3)
                 ON CONFLICT(skill_id) DO UPDATE SET group_id = excluded.group_id",
                params![skill_id, group_id, now_ms()],
            )
            .context("Failed to assign skill to group")?;
        }
        None => {
            tx.execute(
                "DELETE FROM skill_group_members WHERE skill_id = ?1",
                params![skill_id],
            )
            .context("Failed to remove skill from group")?;
        }
    }
    tx.commit()
        .context("Failed to commit skill group assignment")?;
    Ok(())
}

/// Reassign membership from merged-away skill ids onto the surviving skill id.
/// If the survivor already has an assignment, the merged ids' assignments are
/// simply dropped; otherwise the first available one moves onto the survivor.
/// Any remaining rows for the merged ids are removed so they cannot dangle.
pub fn migrate_members(
    conn: &mut Connection,
    from_skill_ids: &[String],
    to_skill_id: &str,
) -> Result<usize> {
    if from_skill_ids.is_empty() {
        return Ok(0);
    }
    let tx = conn
        .transaction()
        .context("Failed to migrate skill group members")?;
    let survivor_has = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM skill_group_members WHERE skill_id = ?1)",
            params![to_skill_id],
            |row| row.get::<_, i64>(0),
        )
        .context("Failed to read survivor skill group membership")?
        != 0;
    let mut moved = 0;
    if !survivor_has {
        for from in from_skill_ids {
            let updated = tx.execute(
                "UPDATE skill_group_members SET skill_id = ?1 WHERE skill_id = ?2",
                params![to_skill_id, from],
            )?;
            if updated > 0 {
                moved += updated;
                break;
            }
        }
    }
    for from in from_skill_ids {
        moved += tx.execute(
            "DELETE FROM skill_group_members WHERE skill_id = ?1",
            params![from],
        )?;
    }
    tx.commit()
        .context("Failed to commit skill group member migration")?;
    Ok(moved)
}

pub fn get_group(conn: &Connection, id: &str) -> Result<SkillGroup> {
    conn.query_row(
        "SELECT id, name, description, color, sort_order, created_at_ms, updated_at_ms
         FROM skill_groups WHERE id = ?1",
        params![id],
        row_to_group,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("Unknown skill group: {id}"))
}

fn row_to_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillGroup> {
    Ok(SkillGroup {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        color: row.get(3)?,
        sort_order: row.get(4)?,
        created_at_ms: row.get(5)?,
        updated_at_ms: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::local_store::LocalSqliteStore;

    fn store() -> LocalSqliteStore {
        let dir = tempfile::tempdir().unwrap();
        LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap()
    }

    #[test]
    fn create_list_update_delete_group_and_assign_skills() {
        let mut store = store();
        let frontend = create_group(
            store.connection_mut(),
            "Frontend",
            Some("UI skills"),
            Some("#6366f1"),
            0,
        )
        .unwrap();
        let testing = create_group(store.connection_mut(), "Testing", None, None, 1).unwrap();

        let groups = list_groups(store.connection()).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].group.name, "Frontend");
        assert_eq!(groups[0].member_count, 0);
        assert_eq!(groups[1].group.name, "Testing");

        // Assign two skills — one skill flips from frontend to testing, proving
        // the one-group-per-skill invariant holds.
        set_skill_group(store.connection_mut(), "skill:react", Some(&frontend.id)).unwrap();
        set_skill_group(store.connection_mut(), "skill:vue", Some(&frontend.id)).unwrap();
        set_skill_group(store.connection_mut(), "skill:react", Some(&testing.id)).unwrap();

        let groups = list_groups(store.connection()).unwrap();
        let frontend_members = groups
            .iter()
            .find(|group| group.group.id == frontend.id)
            .unwrap();
        let testing_members = groups
            .iter()
            .find(|group| group.group.id == testing.id)
            .unwrap();
        assert_eq!(
            frontend_members.member_skill_ids,
            vec!["skill:vue".to_string()]
        );
        assert_eq!(
            testing_members.member_skill_ids,
            vec!["skill:react".to_string()]
        );

        // Ungroup clears the assignment.
        set_skill_group(store.connection_mut(), "skill:vue", None).unwrap();
        let groups = list_groups(store.connection()).unwrap();
        let frontend_members = groups
            .iter()
            .find(|group| group.group.id == frontend.id)
            .unwrap();
        assert!(frontend_members.member_skill_ids.is_empty());

        // Update rewrites editable fields and clears optional ones with empty input.
        let updated = update_group(
            store.connection_mut(),
            &frontend.id,
            "Frontend Tools",
            Some(""),
            None,
            5,
        )
        .unwrap();
        assert_eq!(updated.name, "Frontend Tools");
        assert_eq!(updated.sort_order, 5);
        assert!(updated.description.is_none());

        // Deleting a group cascades: its members vanish (skills are not deleted).
        delete_group(store.connection_mut(), &testing.id).unwrap();
        let groups = list_groups(store.connection()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group.id, frontend.id);
    }

    #[test]
    fn set_skill_group_rejects_unknown_group() {
        let mut store = store();
        let result = set_skill_group(
            store.connection_mut(),
            "skill:react",
            Some("group:does-not-exist"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn migrate_members_moves_assignment_onto_survivor() {
        let mut store = store();
        let group = create_group(store.connection_mut(), "G", None, None, 0).unwrap();
        set_skill_group(
            store.connection_mut(),
            "skill:writer-codex",
            Some(&group.id),
        )
        .unwrap();

        // Consolidate merged `skill:writer-codex` into `skill:writer-claude`.
        let moved = migrate_members(
            store.connection_mut(),
            &["skill:writer-codex".to_string()],
            "skill:writer-claude",
        )
        .unwrap();
        assert_eq!(moved, 1);

        let groups = list_groups(store.connection()).unwrap();
        assert_eq!(
            groups[0].member_skill_ids,
            vec!["skill:writer-claude".to_string()]
        );
    }

    #[test]
    fn migrate_members_drops_when_survivor_already_assigned() {
        let mut store = store();
        let group = create_group(store.connection_mut(), "G", None, None, 0).unwrap();
        set_skill_group(
            store.connection_mut(),
            "skill:writer-claude",
            Some(&group.id),
        )
        .unwrap();
        set_skill_group(
            store.connection_mut(),
            "skill:writer-codex",
            Some(&group.id),
        )
        .unwrap();

        let moved = migrate_members(
            store.connection_mut(),
            &["skill:writer-codex".to_string()],
            "skill:writer-claude",
        )
        .unwrap();
        // Survivor already has an assignment, so the merged id's row is dropped
        // (not double-counted): one delete counts as moved.
        assert_eq!(moved, 1);
        let groups = list_groups(store.connection()).unwrap();
        assert_eq!(
            groups[0].member_skill_ids,
            vec!["skill:writer-claude".to_string()]
        );
    }
}
