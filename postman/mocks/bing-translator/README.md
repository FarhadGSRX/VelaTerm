# Bing Translator HTTP fixture

This local fixture models the page credentials, form fields and response shape used by
[Bing Translator](https://www.bing.com/translator). It deliberately expires the first
page token, then accepts the refreshed token and translates two known English fragments.
Unknown endpoints return 501 and unexpected request fields return 400.

The fixture is maintained here because the Postman CLI is signed out, so an owner mock
could not be searched. A simulation pass validates the Rust HTTP client, credential
refresh, language mapping and protected-text restoration; it does not establish live
Bing availability.

Run the simulation from the repository root, then run the ignored integration test:

```sh
postman simulation run postman/simulations/translation-bing.sim.yaml --port 4918 --output ndjson
VLX_BING_TEST_BASE=http://127.0.0.1:4918/bing cargo test --manifest-path src-tauri/Cargo.toml --lib --no-default-features bing_http_simulation -- --ignored
```

The override is read only by this test; production uses the fixed Bing origin.
