use std::collections::HashSet;
use std::iter::once;
use std::path::PathBuf;

use bytes::BytesMut;
use docker_credential::DockerCredential;
use oci_client::manifest::{OciDescriptor, OciManifest};
use oci_client::secrets::RegistryAuth;
use oci_spec::distribution::Reference;
use oci_spec::image::ImageConfiguration;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::builder::Builder;
use crate::fs::os::OsSource;
use crate::fs::tar::TarDestination;
use crate::io::HashedWriter;

/// Build an OCI image from files or a tar archive.
#[derive(Clone, Debug, clap::Parser)]
pub(crate) struct Args {
    /// Path to the source directory.
    source: PathBuf,

    /// Base image to build an image layer on top of.
    /// It must be already pushed to the same registry as the output.
    #[clap(long)]
    from: Reference,

    /// Image identifier(s) for the output.
    #[clap(short, long = "tag")]
    tags: Vec<Reference>,

    /// Directory in the image to copy the files into (defaults to /).
    #[clap(short, long)]
    root_dir: Option<String>,

    /// Change the owner UID of files copied to the image (defaults to 0).
    #[clap(long)]
    uid: Option<u64>,

    /// Change the owner GID of files copied to the image (defaults to 0).
    #[clap(long)]
    gid: Option<u64>,

    /// Push the built image to the registry (disabled by default).
    #[clap(long)]
    push: bool,
}

pub(super) async fn run(args: Args) -> anyhow::Result<()> {
    let client_config = oci_client::client::ClientConfig {
        // Some registries don't support chunked push.
        use_monolithic_push: true,
        ..Default::default()
    };

    let client = oci_client::Client::new(client_config);

    let registries = once(args.from.resolve_registry())
        .chain(args.tags.iter().map(|tag| tag.resolve_registry()))
        .collect::<HashSet<_>>();

    authenticate_registries(&client, &registries).await;

    // Pull the manifest and image config of the base image.
    let (mut manifest, _digest, image_config) = client
        .pull_manifest_and_config(&args.from, &get_registry_auth(args.from.resolve_registry()))
        .await?;

    // Parse the image configuration.
    let mut image_config = ImageConfiguration::from_reader(image_config.as_bytes())?;

    // Build the image layer.
    let mut buffer = BytesMut::new();
    let (diff_id, digest) = {
        // Build the source pipe.
        let source = OsSource::new(args.source);

        // Build the destination pipe.
        // Note that Diff ID is calculated from the uncompressed buffer, while Digest is calculated
        // from the compressed one.
        let mut digest_writer = HashedWriter::new(Sha256::new(), buffer.as_mut());
        let mut diff_id_writer =
            HashedWriter::new(Sha256::new(), zstd::Encoder::new(&mut digest_writer, 0)?);
        let destination = TarDestination::new(
            &mut diff_id_writer,
            args.uid.unwrap_or(0),
            args.gid.unwrap_or(0),
        );

        Builder::new(
            source,
            destination,
            args.root_dir.unwrap_or_else(|| "/".to_string()),
        )
        .build()?;

        (
            format_digest(diff_id_writer.finalize().into()),
            format_digest(digest_writer.finalize().into()),
        )
    };

    // Append the Diff ID to the image config.
    image_config.rootfs_mut().diff_ids_mut().push(diff_id);

    let image_config = image_config.to_string()?;
    let image_config_digest = format_digest(Sha256::digest(image_config.as_bytes()).into());

    manifest.config.size = image_config.len() as i64;
    manifest.config.digest = image_config_digest.clone();

    // Append a layer to the manifest.
    manifest.layers.push(OciDescriptor {
        media_type: "application/vnd.oci.image.layer.v1.tar+zstd".to_string(),
        size: buffer.len() as i64,
        digest: digest.clone(),
        ..Default::default()
    });

    let buffer = buffer.freeze();
    let manifest = OciManifest::Image(manifest);

    if args.push {
        for tag in &args.tags {
            info!(%tag, "Pushing image configuration");

            client
                .push_blob(tag, image_config.clone(), &image_config_digest)
                .await?;

            info!(%tag, "Pushing image");

            client.push_blob(tag, buffer.clone(), &digest).await?;

            info!(%tag, "Pushing manifest");

            client.push_manifest(tag, &manifest).await?;
        }
    } else {
        warn!("Pushing is disabled, skipping");
    }

    Ok(())
}

fn get_registry_auth(registry: &str) -> RegistryAuth {
    match docker_credential::get_credential(registry) {
        Ok(DockerCredential::UsernamePassword(username, password)) => {
            RegistryAuth::Basic(username, password)
        }
        Ok(DockerCredential::IdentityToken(token)) => RegistryAuth::Bearer(token),
        _ => RegistryAuth::Anonymous,
    }
}

async fn authenticate_registries(client: &oci_client::Client, registries: &HashSet<&str>) {
    info!("Authenticating registries");

    for registry in registries {
        let auth = get_registry_auth(registry);

        client.store_auth_if_needed(registry, &auth).await;

        if matches!(auth, RegistryAuth::Anonymous) {
            warn!(registry, "No credentials found, continuing as anonymous");
        } else {
            info!(registry, "Successfully authenticated");
        }
    }
}

fn format_digest(digest: [u8; 32]) -> String {
    format!("sha256:{}", hex::encode(digest))
}
