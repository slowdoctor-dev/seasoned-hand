//! Story 5.4 regression tests.
//! refs: /specs/phase-5/stories/story-5.4.md

use super::*;
use crate::auth::Role;
use crate::db;
use rusqlite::params;

const TENANT: &str = "tenant-test";

async fn setup() -> (
    DbPool,
    OrganizationStore,
    UserStore,
    MembershipStore,
    ProjectRoleOverrideStore,
) {
    let pool = db::open(":memory:").await.unwrap();
    (
        pool.clone(),
        OrganizationStore::new(pool.clone()),
        UserStore::new(pool.clone()),
        MembershipStore::new(pool.clone()),
        ProjectRoleOverrideStore::new(pool),
    )
}

#[tokio::test]
async fn organization_store_round_trip() {
    let (_db, orgs, _, _, _) = setup().await;
    let id = orgs
        .insert(NewOrganization {
            tenant_id: TENANT.into(),
            slug: "acme".into(),
            display_name: "Acme Corp".into(),
        })
        .await
        .expect("insert");
    let got = orgs.get(&id).await.expect("get");
    assert_eq!(got.tenant_id, TENANT);
    assert_eq!(got.slug, "acme");
    assert_eq!(got.display_name, "Acme Corp");
    assert_eq!(got.status, "active");

    // The V013 sentinel + the one we just inserted (2 rows in this tenant).
    let list = orgs.list_by_tenant(TENANT).await.expect("list");
    assert_eq!(list.len(), 1);
    let legacy = orgs
        .list_by_tenant("legacy-default")
        .await
        .expect("list legacy");
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].slug, "legacy-default");
}

#[tokio::test]
async fn user_store_insert_get_and_deactivate() {
    let (_db, _, users, _, _) = setup().await;
    let id = users
        .insert(NewUser {
            tenant_id: TENANT.into(),
            email: "alice@example.com".into(),
            display_name: "Alice".into(),
        })
        .await
        .expect("insert");
    assert_eq!(users.get(&id).await.unwrap().status, "active");
    users.soft_deactivate(&id).await.expect("deactivate");
    assert_eq!(users.get(&id).await.unwrap().status, "deactivated");

    // Deactivate of a non-existent user surfaces NotFound, not a silent no-op.
    let err = users
        .soft_deactivate("user-nonexistent")
        .await
        .expect_err("non-existent must fail");
    assert!(matches!(err, OrgStoreError::NotFound(_)));
}

#[tokio::test]
async fn membership_primary_per_user_unique_invariant() {
    // The V013 partial unique index `idx_membership_primary_per_user`
    // (WHERE is_primary = 1) enforces "at most one primary membership per
    // user" across the entire memberships table.
    //
    // Architecture §3.2 pins `organizations.tenant_id UNIQUE` (1 tenant = 1 org),
    // so the two orgs in this test live in two distinct tenant namespaces.
    // The user belongs to tenant A; the secondary membership in org B carries
    // org B's tenant_id (per FK chain).
    let (_db, orgs, users, memberships, _) = setup().await;
    let org_a = orgs
        .insert(NewOrganization {
            tenant_id: "tenant-a".into(),
            slug: "a".into(),
            display_name: "A".into(),
        })
        .await
        .unwrap();
    let org_b = orgs
        .insert(NewOrganization {
            tenant_id: "tenant-b".into(),
            slug: "b".into(),
            display_name: "B".into(),
        })
        .await
        .unwrap();
    let user = users
        .insert(NewUser {
            tenant_id: "tenant-a".into(),
            email: "bob@example.com".into(),
            display_name: "Bob".into(),
        })
        .await
        .unwrap();
    memberships
        .insert(NewMembership {
            tenant_id: "tenant-a".into(),
            organization_id: org_a.clone(),
            user_id: user.clone(),
            role: Role::User,
            is_primary: true,
        })
        .await
        .expect("first primary");
    let err = memberships
        .insert(NewMembership {
            tenant_id: "tenant-b".into(),
            organization_id: org_b.clone(),
            user_id: user.clone(),
            role: Role::User,
            is_primary: true,
        })
        .await
        .expect_err("second primary must fail");
    assert!(matches!(err, OrgStoreError::Sqlite(_)));

    // But a secondary (is_primary=false) membership is allowed.
    memberships
        .insert(NewMembership {
            tenant_id: "tenant-b".into(),
            organization_id: org_b,
            user_id: user.clone(),
            role: Role::Viewer,
            is_primary: false,
        })
        .await
        .expect("secondary ok");
    let listed = memberships.list_for_user(&user).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed[0].is_primary, "primary first");
    assert!(!listed[1].is_primary, "secondary follows");
}

#[tokio::test]
async fn membership_update_role_round_trip() {
    let (_db, orgs, users, memberships, _) = setup().await;
    let org = orgs
        .insert(NewOrganization {
            tenant_id: TENANT.into(),
            slug: "acme".into(),
            display_name: "Acme".into(),
        })
        .await
        .unwrap();
    let user = users
        .insert(NewUser {
            tenant_id: TENANT.into(),
            email: "c@e.com".into(),
            display_name: "C".into(),
        })
        .await
        .unwrap();
    let membership = memberships
        .insert(NewMembership {
            tenant_id: TENANT.into(),
            organization_id: org,
            user_id: user.clone(),
            role: Role::Viewer,
            is_primary: true,
        })
        .await
        .unwrap();
    memberships
        .update_role(&membership, Role::Admin)
        .await
        .expect("update");
    let updated = memberships
        .list_for_user(&user)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(updated.role, Role::Admin);
}

#[tokio::test]
async fn project_role_override_precedence() {
    // F-5.4 / architecture §4.1: project override role wins over org role
    // when querying effective role for a (user, project) pair.
    let (db, _orgs, users, _memberships, overrides) = setup().await;
    // Seed a project row directly (V006 schema; we don't need ProjectStore here).
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, description, status, created_at, updated_at)
             VALUES ('proj-x', ?, 'X', NULL, 'active', 0, 0)",
            params![TENANT],
        )
        .unwrap();
        Ok::<(), OrgStoreError>(())
    })
    .await
    .unwrap();
    let user = users
        .insert(NewUser {
            tenant_id: TENANT.into(),
            email: "d@e.com".into(),
            display_name: "D".into(),
        })
        .await
        .unwrap();
    overrides
        .insert(NewProjectRoleOverride {
            tenant_id: TENANT.into(),
            project_id: "proj-x".into(),
            user_id: user.clone(),
            role: Role::Admin,
        })
        .await
        .expect("override insert");
    let role = overrides.for_user_project(&user, "proj-x").await.unwrap();
    assert_eq!(role, Some(Role::Admin));

    // No row for a different project returns None (org role takes over).
    let none = overrides
        .for_user_project(&user, "proj-other")
        .await
        .unwrap();
    assert_eq!(none, None);
}
