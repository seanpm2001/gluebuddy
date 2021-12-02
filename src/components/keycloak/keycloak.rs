//! This module defines keycloak related actions and enforcements.
//!
//! ## Features
//!

use crate::args::Action;

use keycloak::types::{CredentialRepresentation, GroupRepresentation, UserRepresentation};
use keycloak::{KeycloakAdmin, KeycloakAdminToken};
use reqwest::Client;

use futures::future::try_join_all;

use anyhow::{Context, Result};
use log::{debug, info};
use tokio::sync::Mutex;

use std::env;
use std::sync::Arc;

use crate::state::State;
use crate::state::User;

pub struct Keycloak {
    admin: KeycloakAdmin,
    realm: String,
    state: Arc<Mutex<State>>,
}

impl Keycloak {
    pub async fn new(state: Arc<Mutex<State>>) -> Result<Keycloak> {
        let username = &env::var("GLUEBUDDY_KEYCLOAK_USERNAME")
            .context("Missing env var GLUEBUDDY_KEYCLOAK_USERNAME")?;
        let password = &env::var("GLUEBUDDY_KEYCLOAK_PASSWORD")
            .context("Missing env var GLUEBUDDY_KEYCLOAK_PASSWORD")?;
        let realm = &env::var("GLUEBUDDY_KEYCLOAK_REALM")
            .context("Missing GLUEBUDDY_KEYCLOAK_REALM env var")?;
        let url = &env::var("GLUEBUDDY_KEYCLOAK_URL")
            .context("Missing GLUEBUDDY_KEYCLOAK_URL env var")?;

        let client = Client::new();

        info!(
            "acquire API token for keycloak {} using realm {}",
            url, realm
        );
        let token = KeycloakAdminToken::acquire(url, username, password, &client).await?;
        let admin = KeycloakAdmin::new(url, token, client);

        Ok(Keycloak {
            admin,
            realm: realm.to_string(),
            state,
        })
    }

    pub async fn gather(&self) -> Result<()> {
        info!("Gathering Keycloak state");
        let root_groups = vec!["Arch Linux Staff", "External Contributors"];

        let all_groups = self
            .admin
            .realm_groups_get(&self.realm, None, None, None, None)
            .await?;
        let groups = all_groups
            .iter()
            .filter(|group| root_groups.contains(&group.name.as_ref().unwrap().as_ref()))
            .collect::<Vec<_>>();

        let groups_members = groups.into_iter().flat_map(|group| {
            let group_name = group.name.as_ref().unwrap();
            info!(
                "collect members of group {} via {}",
                group_name,
                group.path.as_ref().unwrap()
            );
            vec![Box::pin(get_group_members(
                &self.admin,
                &self.realm,
                group.clone(),
            ))]
            .into_iter()
            .chain(group.sub_groups.as_ref().unwrap().iter().map(|sub_group| {
                info!(
                    "collect members of sub group {} via {}",
                    sub_group.name.as_ref().unwrap(),
                    sub_group.path.as_ref().unwrap()
                );
                Box::pin(get_group_members(
                    &self.admin,
                    &self.realm,
                    sub_group.clone(),
                ))
            }))
            .chain(
                group
                    .sub_groups
                    .as_ref()
                    .unwrap()
                    .iter()
                    .flat_map(|sub_group| sub_group.sub_groups.as_ref().unwrap())
                    .map(|sub_group| {
                        info!(
                            "collect members of sub group {} via {}",
                            sub_group.name.as_ref().unwrap(),
                            sub_group.path.as_ref().unwrap(),
                        );
                        Box::pin(get_group_members(
                            &self.admin,
                            &self.realm,
                            sub_group.clone(),
                        ))
                    }),
            )
        });

        let group_members = try_join_all(groups_members).await?;
        let mut state = self.state.lock().await;

        for (group, users) in group_members {
            for user in users {
                let group_name = group.name.as_ref().unwrap();
                let path = group.path.as_ref().unwrap();
                debug!(
                    "group {} via {} user {}",
                    group_name,
                    path,
                    user.username.as_ref().unwrap()
                );

                let state_user = state
                    .users
                    .entry(user.username.as_ref().unwrap().to_string())
                    .or_insert_with_key(|key| User::new(key.clone()));
                state_user.groups.insert(path.to_string());
            }
        }

        Ok(())
    }

    pub async fn run(&self, action: Action) -> Result<()> {
        Ok(())
    }
}

async fn get_group_members<'a>(
    admin: &'a KeycloakAdmin,
    realm: &'a str,
    group: GroupRepresentation,
) -> Result<(GroupRepresentation, Vec<UserRepresentation>)> {
    let users = admin
        .realm_groups_with_id_members_get(
            realm,
            group.id.as_ref().unwrap().as_ref(),
            None,
            None,
            None,
        )
        .await?;
    Ok((group, users))
}
