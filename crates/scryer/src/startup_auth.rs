use scryer_application::AppUseCase;

pub(crate) struct BootstrapAdmin {
    pub username: Option<String>,
    pub password: Option<String>,
    pub reset: bool,
    pub disable_default: bool,
}

impl BootstrapAdmin {
    pub fn active(&self) -> bool {
        self.username.is_some() || self.password.is_some() || self.reset || self.disable_default
    }
}

fn read_env(name: &str) -> Result<Option<String>, String> {
    std::env::var(name).map(Some).or_else(|error| match error {
        std::env::VarError::NotPresent => Ok(None),
        _ => Err(format!("{name} must contain valid text")),
    })
}

fn bootstrap_bool(name: &str, value: Option<&str>) -> Result<bool, String> {
    match value {
        None => Ok(false),
        Some(value) => super::parse_env_bool_value(value).ok_or_else(|| {
            format!("{name} must be a valid boolean; omit the variable instead of leaving it blank")
        }),
    }
}

pub(crate) fn read_bootstrap_admin() -> Result<BootstrapAdmin, String> {
    let config = BootstrapAdmin {
        username: read_env("SCRYER_ADMIN_USERNAME")?,
        password: read_bootstrap_password()?,
        reset: bootstrap_bool(
            "SCRYER_ADMIN_PASSWORD_RESET",
            read_env("SCRYER_ADMIN_PASSWORD_RESET")?.as_deref(),
        )?,
        disable_default: bootstrap_bool(
            "SCRYER_DISABLE_DEFAULT_ADMIN",
            read_env("SCRYER_DISABLE_DEFAULT_ADMIN")?.as_deref(),
        )?,
    };
    if config.reset && config.password.is_none() {
        return Err("SCRYER_ADMIN_PASSWORD_RESET requires SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE".into());
    }
    Ok(config)
}

pub(crate) fn read_bootstrap_password() -> Result<Option<String>, String> {
    let file = read_env("SCRYER_ADMIN_PASSWORD_FILE")?;
    let direct = if file.is_none() {
        read_env("SCRYER_ADMIN_PASSWORD")?
    } else {
        None
    };
    resolve_bootstrap_password(direct, file, |path| {
        std::fs::read_to_string(path)
            .map_err(|_| "cannot read SCRYER_ADMIN_PASSWORD_FILE as text".to_string())
    })
}

fn resolve_bootstrap_password(
    direct: Option<String>,
    file: Option<String>,
    read_file: impl FnOnce(&str) -> Result<String, String>,
) -> Result<Option<String>, String> {
    let password = if let Some(path) = file {
        if path.trim().is_empty() {
            return Err("SCRYER_ADMIN_PASSWORD_FILE must name a readable secret file; omit the variable instead of leaving it blank".into());
        }
        Some(read_file(&path)?.trim_end_matches(['\r', '\n']).to_string())
    } else {
        direct
    };
    if password
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(
            "SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE must contain a nonempty password; omit the variables to leave bootstrap unconfigured"
                .into(),
        );
    }
    Ok(password)
}

pub(crate) async fn ensure_admin_password_configured(
    app_use_case: &AppUseCase,
) -> Result<(), String> {
    if app_use_case
        .existing_default_admin_uses_bootstrap_password()
        .await
        .map_err(|error| format!("failed to validate default admin password state: {error}"))?
    {
        return Err(
            "form login is enabled, but the default admin password is still 'admin'; change it before enabling auth".to_string(),
        );
    }

    if !app_use_case
        .usable_admin_login_exists()
        .await
        .map_err(|error| format!("failed to validate admin login state: {error}"))?
    {
        return Err(
            "form login is enabled, but no local full-admin user has a usable password; initialize a fresh admin with SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE, or use SCRYER_RECOVERY_ADMIN_PASSWORD to recover an existing instance".to_string(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_bootstrap_password;

    #[test]
    fn bootstrap_flags_reject_invalid_values() {
        assert!(!super::bootstrap_bool("flag", None).unwrap());
        assert!(super::bootstrap_bool("flag", Some("true")).unwrap());
        assert!(!super::bootstrap_bool("flag", Some("false")).unwrap());
        assert!(super::bootstrap_bool("flag", Some("maybe")).is_err());
        assert!(super::bootstrap_bool("flag", Some("")).is_err());
    }

    #[test]
    fn bootstrap_password_preserves_direct_secret() {
        assert_eq!(
            resolve_bootstrap_password(
                Some(" temporary password ".into()),
                None,
                |_| unreachable!()
            )
            .unwrap(),
            Some(" temporary password ".into())
        );
        assert_eq!(
            resolve_bootstrap_password(None, None, |_| unreachable!()).unwrap(),
            None
        );
    }

    #[test]
    fn bootstrap_password_file_wins_and_only_strips_line_endings() {
        assert_eq!(
            resolve_bootstrap_password(Some("ignored".into()), Some("secret".into()), |_| Ok(
                " temporary password \r\n".into()
            ))
            .unwrap(),
            Some(" temporary password ".into())
        );
        assert!(
            resolve_bootstrap_password(Some("ignored".into()), Some("secret".into()), |_| Err(
                "unreadable".into()
            ))
            .is_err()
        );
    }

    #[test]
    fn bootstrap_password_rejects_empty_secrets() {
        assert!(resolve_bootstrap_password(Some(" ".into()), None, |_| unreachable!()).is_err());
        assert!(
            resolve_bootstrap_password(None, Some("secret".into()), |_| Ok("\n".into())).is_err()
        );
        assert!(resolve_bootstrap_password(None, Some("".into()), |_| unreachable!()).is_err());
    }
}
