// @generated automatically by Diesel CLI.

diesel::table! {
    servers (name) {
        name -> Text,
        host -> Text,
    }
}
