use scryer_domain::{
    ListAccountCredential, ListScope, User, UserListAccount, UserListAccountStatus,
};

use super::*;
use crate::lists::test_support::{PROVIDER, at, subscription};

fn member(id: &str) -> User {
    User {
        id: id.to_string(),
        ..User::new_admin(format!("member-{id}"))
    }
}

fn personal(owner: &str) -> ListSubscription {
    ListSubscription {
        scope: ListScope::Personal,
        owner_user_id: owner.to_string(),
        credential_id: Some("account-one".to_string()),
        ..subscription("personal-list")
    }
}

fn account(user_id: &str, provider: &str, status: UserListAccountStatus) -> UserListAccount {
    UserListAccount {
        id: "account-one".to_string(),
        user_id: user_id.to_string(),
        provider: provider.to_string(),
        external_user_id: "external-one".to_string(),
        username: "fixture-member".to_string(),
        display_name: None,
        credential: ListAccountCredential {
            access_token: "fixture-token".to_string(),
            ..ListAccountCredential::default()
        },
        status,
        error_message: None,
        linked_at: at(0),
        last_used_at: None,
        last_refresh_at: None,
        updated_at: at(0),
    }
}

#[test]
fn a_personal_list_is_visible_to_its_owner_only_and_admins_get_not_found() {
    let list = personal("owner-one");
    assert!(ensure_subscription_visible(&member("owner-one"), &list).is_ok());

    // `member` is built from a full administrator: holding every grant does
    // not open another member's personal list.
    let admin = member("admin-one");
    assert!(matches!(
        ensure_subscription_visible(&admin, &list),
        Err(AppError::NotFound(_))
    ));
    assert!(visible_subscriptions(&admin, vec![list.clone()]).is_empty());
    assert_eq!(
        visible_subscriptions(&member("owner-one"), vec![list]).len(),
        1
    );
}

#[test]
fn a_public_list_is_visible_to_everyone() {
    assert!(can_view_subscription(
        &member("anyone"),
        &subscription("public-list")
    ));
}

#[test]
fn another_members_account_is_not_found() {
    let foreign = account("owner-two", PROVIDER, UserListAccountStatus::Active);
    assert!(matches!(
        ensure_account_owner(&member("owner-one"), &foreign),
        Err(AppError::NotFound(_))
    ));
}

#[test]
fn a_credential_needs_the_owners_active_account_for_the_same_provider() {
    let list = personal("owner-one");

    let credential = credential_for(
        &list,
        Some(&account(
            "owner-one",
            PROVIDER,
            UserListAccountStatus::Active,
        )),
    )
    .expect("the owner's active account");
    assert_eq!(credential.access_token, "fixture-token");

    for wrong in [
        account("owner-two", PROVIDER, UserListAccountStatus::Active),
        account("owner-one", "other-provider", UserListAccountStatus::Active),
    ] {
        let failure = credential_for(&list, Some(&wrong)).expect_err("mismatched account");
        assert_eq!(failure.class, ListFailureClass::AccountRequired);
    }
    assert_eq!(
        credential_for(&list, None).expect_err("no account").class,
        ListFailureClass::AccountRequired
    );

    let revoked = account("owner-one", PROVIDER, UserListAccountStatus::Revoked);
    assert_eq!(
        credential_for(&list, Some(&revoked))
            .expect_err("revoked account")
            .class,
        ListFailureClass::Unauthorized
    );
}

#[test]
fn a_personal_failure_label_names_only_owner_provider_and_class() {
    let list = personal("owner-one");
    let label = job_failure_label(
        &list,
        &ListFailure::new(ListFailureClass::Unauthorized, PROVIDER),
    );
    assert!(label.contains("owner-one"));
    assert!(label.contains(PROVIDER));
    assert!(!label.contains(&list.name), "{label}");
    assert!(!label.contains(&list.id), "{label}");
}
