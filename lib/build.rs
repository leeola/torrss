fn main() {
    // The Tailwind CLI comes from the dev shell. A downloaded release links
    // against a dynamic loader that NixOS does not provide.
    topcoat::tailwind::BuildConfig::new()
        .executable("tailwindcss")
        .render()
        .expect("render the Tailwind stylesheet");
}
