/// Interpreter pin changes. An outer `None` keeps the current pin;
/// `Some(None)` or a blank value clears it so the conventional command name
/// is used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateScriptInterpreterSettings {
    pub python: Option<Option<String>>,
    pub powershell: Option<Option<String>>,
    pub batch: Option<Option<String>>,
    pub go: Option<Option<String>>,
}

/// A pin is either an absolute path or a bare command name looked up on
/// `PATH`; a relative path would resolve against each script's own working
/// directory.
fn validate_script_interpreter_pin(key_name: &str, value: &str) -> AppResult<()> {
    let is_bare_name = !value.contains(['/', '\\']);
    if is_bare_name || Path::new(value).is_absolute() {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "{key_name} must be an absolute path or a command name without path separators"
    )))
}

const SCRIPT_INTERPRETER_KEYS: [&str; 4] = [
    crate::SCRIPT_INTERPRETER_PYTHON_KEY,
    crate::SCRIPT_INTERPRETER_POWERSHELL_KEY,
    crate::SCRIPT_INTERPRETER_BATCH_KEY,
    crate::SCRIPT_INTERPRETER_GO_KEY,
];

impl AppUseCase {
    /// Interpreters scripts are launched with. A setting that cannot be read
    /// falls back to the conventional command name rather than blocking the
    /// run.
    pub async fn script_interpreter_config(&self) -> crate::scripts::runner::InterpreterConfig {
        match self.load_script_interpreter_config().await {
            Ok(config) => config,
            Err(error) => {
                warn!(error = %error, "failed to read script interpreter settings");
                crate::scripts::runner::InterpreterConfig::default()
            }
        }
    }

    async fn load_script_interpreter_config(
        &self,
    ) -> AppResult<crate::scripts::runner::InterpreterConfig> {
        let [python, powershell, batch, go] = SCRIPT_INTERPRETER_KEYS;
        Ok(crate::scripts::runner::InterpreterConfig {
            python: self.read_script_interpreter(python).await?,
            powershell: self.read_script_interpreter(powershell).await?,
            batch: self.read_script_interpreter(batch).await?,
            go: self.read_script_interpreter(go).await?,
        })
    }

    async fn read_script_interpreter(&self, key_name: &str) -> AppResult<Option<PathBuf>> {
        Ok(
            normalize_optional_string(self.read_setting_string_value(key_name, None).await?)
                .map(PathBuf::from),
        )
    }

    pub async fn get_script_interpreter_settings(
        &self,
        actor: &User,
    ) -> AppResult<crate::scripts::runner::InterpreterConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_script_interpreter_config().await
    }

    pub async fn update_script_interpreter_settings(
        &self,
        actor: &User,
        input: UpdateScriptInterpreterSettings,
    ) -> AppResult<crate::scripts::runner::InterpreterConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let values = [input.python, input.powershell, input.batch, input.go];
        let mut changes = Vec::new();
        for (key_name, value) in SCRIPT_INTERPRETER_KEYS.into_iter().zip(values) {
            let Some(value) = value else {
                continue;
            };
            let value = normalize_optional_string(value);
            if let Some(path) = &value {
                validate_script_interpreter_pin(key_name, path)?;
            }
            changes.push((key_name, value));
        }

        for (key_name, value) in &changes {
            match value {
                Some(path) => {
                    self.upsert_system_setting_json(key_name, path, Some(actor.id.clone()))
                        .await?;
                }
                None => self.delete_system_setting(key_name).await?,
            }
        }

        self.emit_settings_saved(
            actor,
            "script_interpreter_settings",
            None,
            changes
                .iter()
                .map(|(key_name, _)| key_name.to_string())
                .collect(),
        )
        .await;

        self.load_script_interpreter_config().await
    }
}
