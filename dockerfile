# Use the official Rust image as the base image for the build step
FROM rust:latest AS build

# Set the working directory inside the container
WORKDIR /usr/src/app


# Copy the Cargo.toml and Cargo.lock files to the container
COPY migrations ./migrations
COPY Cargo.toml Cargo.lock dummy.rs  ./
CMD ["/bin/sh"]
RUN sed -i 's#src/main.rs#dummy.rs#' Cargo.toml
RUN cargo build --release
RUN sed -i 's#dummy.rs#src/main.rs#' Cargo.toml

# Copy the source code to the container
COPY src ./src

# Build the project
RUN cargo build --release

# Use a smaller image for the runtime step
FROM debian:stable-slim

# Copy the binary from the build step to the runtime container
COPY --from=build /usr/src/app/target/release/mcping-mqtt /usr/local/bin/mcping-mqtt

# Set the command to run when the container starts
CMD ["mcping-mqtt"]
