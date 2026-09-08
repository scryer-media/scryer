#[derive(Default, serde::Serialize)]
pub(crate) struct RulePackAutoUpdateReport {
    pub(crate) updated: Vec<String>,
    pub(crate) failed: Vec<(String, String)>,
}

impl AppUseCase {
    pub(crate) async fn run_scheduled_rule_pack_auto_update(&self) -> RulePackAutoUpdateReport {
        let mut report = RulePackAutoUpdateReport::default();
        let actor = User::system_execution_actor();
        let installations = match self
            .services
            .customization
            .rule_sets
            .list_rule_pack_installations()
            .await
        {
            Ok(installations) => installations,
            Err(error) => {
                report.failed.push(("catalog".into(), error.to_string()));
                return report;
            }
        };
        for installation in installations
            .into_iter()
            .filter(|installation| installation.auto_update)
        {
            let result = async {
                let Some(candidate) = self
                    .rule_pack_update_candidate(
                        &actor,
                        &installation.pack_id,
                        &installation.version,
                        true,
                    )
                    .await?
                else {
                    return Ok::<_, AppError>(false);
                };
                let pack = self
                    .fetch_verified_rule_pack_version(
                        &actor,
                        &installation.pack_id,
                        &candidate.version,
                    )
                    .await?;
                if pack.registry.digest != candidate.digest {
                    return Err(AppError::Validation(
                        "rule pack catalog changed during download".into(),
                    ));
                }
                self.apply_verified_tracked_pack_update(&actor, &pack, installation.revision, true)
                    .await?;
                Ok(true)
            }
            .await;
            match result {
                Ok(true) => report.updated.push(installation.pack_id),
                Ok(false) => {}
                Err(error) => {
                    // Do not overwrite a user's newer settings, a successful
                    // interactive update, or an uninstall while downloading.
                    let _mutation = self.services.customization.rule_mutation_lock.lock().await;
                    let record = async {
                        let Some(mut current) = self
                            .services
                            .customization
                            .rule_sets
                            .get_rule_pack_installation(&installation.pack_id)
                            .await?
                        else {
                            return Ok::<_, AppError>(false);
                        };
                        if current.revision != installation.revision || !current.auto_update {
                            return Ok(false);
                        }
                        current.last_error = Some(error.to_string());
                        current.revision += 1;
                        self.services
                            .customization
                            .rule_sets
                            .apply_rule_pack_installation(
                                &current,
                                Some(installation.revision),
                                &[],
                                &[],
                            )
                            .await
                    }
                    .await;
                    match record {
                        Ok(false) => {} // A concurrent change superseded this run.
                        Ok(true) => report
                            .failed
                            .push((installation.pack_id, error.to_string())),
                        Err(persist_error) => report.failed.push((
                            installation.pack_id,
                            format!("{error}; could not save update status: {persist_error}"),
                        )),
                    }
                }
            }
        }
        report
    }
}
