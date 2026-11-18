//! Refresh-token rotation must not consume credentials if issuance fails.

use anyhow::{Context, ensure};
use chrono::{Duration, Utc};
use e2e_tests::common::TestEnv;
use gateway_example::auth::refresh::{
    generate_raw_refresh_token, hash_refresh_token, prepare_rotation,
};
use rwf_domain::UserId;
use uuid::Uuid;

#[tokio::test]
async fn dropping_prepared_rotation_rolls_back_both_changes() -> anyhow::Result<()> {
    let db = TestEnv::postgres().await?;
    let (old_raw, old_jti) = generate_raw_refresh_token()?;
    let subject_id = Uuid::new_v4();
    let subject = UserId::try_from(subject_id)?;
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO refresh_tokens (jti, family_id, subject, hashed_token, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(old_jti)
    .bind(old_jti)
    .bind(subject_id)
    .bind(hash_refresh_token(&old_raw).to_vec())
    .bind(now + Duration::minutes(10))
    .bind(now)
    .execute(db.pool())
    .await?;

    // Simulates access-token signing failure after a successful DB rotation
    // preparation: the uncommitted transaction must undo both writes.
    let pending = prepare_rotation(db.pool(), &old_raw, now, 60)
        .await?
        .context("initial refresh should be active")?;
    ensure!(pending.subject() == subject, "wrong subject on rotation");
    drop(pending);

    // If the initial rotation had committed, this same token would instead
    // trigger replay detection and invalidate the entire refresh family.
    let retry = prepare_rotation(db.pool(), &old_raw, now, 60)
        .await?
        .context("old refresh token must remain usable after rollback")?;
    let new_token = retry.commit().await?;
    ensure!(
        new_token.subject == subject,
        "wrong subject on committed rotation"
    );
    ensure!(
        new_token.new_raw_token != old_raw,
        "refresh token was not replaced"
    );

    let (old_revoked, new_active): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM refresh_tokens WHERE jti = $1 AND revoked_at IS NOT NULL), \
                EXISTS(SELECT 1 FROM refresh_tokens WHERE hashed_token = $2 AND revoked_at IS NULL)",
    )
    .bind(old_jti)
    .bind(hash_refresh_token(&new_token.new_raw_token).to_vec())
    .fetch_one(db.pool())
    .await?;
    ensure!(
        old_revoked && new_active,
        "committed rotation state is inconsistent"
    );
    Ok(())
}
