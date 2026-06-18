# ocinoco - Build <ins>**OCI**</ins> Image with <ins>**no**</ins> <ins>**co**</ins>ntainer

_English_ | [日本語](README.ja.md)

> [!WARNING]
> This project is under development. Please validate it thoroughly before using it in production environments.
> The developer assumes no responsibility for any damages or losses resulting from its use.

## Motivation

OCI images, also known as Docker images, are typically built with toolkits such as BuildKit,
which start a build container to perform the build. This is an excellent mechanism for reproducible builds,
but it takes time.

The time spent during a build can be broken down as follows, assuming it runs on CI:

1. Set up BuildKit
2. Download the base image and cache
3. Start the build container
4. Run the build (`pnpm install`, `tsc`, etc.)
5. Create an OCI image from the container
6. Push it to a registry

Especially when building applications written in scripting languages such as JavaScript, TypeScript, Python, and Ruby,
the steps other than the build step itself tend to account for a large share of the total time.

**ocinoco** eliminates most of these steps:

1. Run the build (`pnpm install`, `tsc`, etc.)
2. Create an OCI image from the base image and filesystem
3. Push it to a registry

As the mechanism described below shows, ocinoco does not even need to download the base image.
As long as you have an existing built base image and the required files, you can **create an OCI image in seconds**.

## How It Works

An OCI image consists of multiple **layers**. By stacking layers that represent differences in files and directories,
an image represents the final filesystem inside the container.

A common image layer structure looks like this, in simplified form:

| Layer | Contents         |
|-------|------------------|
| 3     | Application code |
| 2     | Language runtime |
| 1     | Middleware       |

When building an application, the only thing that usually needs to be created is the topmost layer.
In other words, building an image can be described as creating a new top layer and combining it with a base image.

Layers also exist independently in registries, where they are called blobs. To create an image with an added layer,
you only need to upload the blob for the new layer and the modified manifest.

Based on this premise, ocinoco provides a build method that minimizes the required steps by converting application code
files on the filesystem into an OCI-compliant layer and pushing it to a registry.

## Installation

At the moment, ocinoco is distributed only through crates.io.
npm distribution is also planned.

```shell
cargo install ocinoco
```

## Usage

> [!IMPORTANT]
> Make sure the environment used for the build matches the image's runtime environment (platform).
> For example, some npm packages resolve native binaries during installation,
> so building in an environment that differs from the runtime may cause them to stop working.

### Create an OCI Image

The `build` command creates an OCI layer in `tar.zst` format from the specified directory, then creates an OCI image
by adding that layer to an existing base image.

```shell
ocinoco build ./dist \
  --from ghcr.io/example/base:latest \
  --tag ghcr.io/example/app:latest
```

By default, the created image is not pushed to a registry. Specify `--push` to push it.

```shell
ocinoco build ./dist \
  --from ghcr.io/example/base:latest \
  --tag ghcr.io/example/app:latest \
  --push
```

> [!NOTE]
> The specified base image must already exist in the registry.

To specify the destination path or ownership inside the archive, use options like the following:

```shell
ocinoco build ./dist \
  --from ghcr.io/example/base:latest \
  --tag ghcr.io/example/app:latest \
  --root-dir /app \
  --uid 1000 \
  --gid 1000
```

### Create a tar.zst Archive

The `pack` command outputs the specified directory as a `tar.zst` archive in the same format as an OCI layer.
It does not connect to a registry or create an OCI manifest.

```shell
ocinoco pack ./dist ./layer.tzst
```

To specify the destination path or ownership inside the archive, use options like the following:

```shell
ocinoco pack ./dist ./layer.tzst \
  --root-dir /app \
  --uid 1000 \
  --gid 1000
```
