# Vertex test fixtures

Two throwaway RSA-2048 private keys, used only by `src/vertex/tests.rs` to
prove that `VertexProvider::signed_assertion` accepts **both** PEM encodings
Google hands out for a service account:

| file | encoding | PEM tag |
| --- | --- | --- |
| `service-account-pkcs1.pem` | PKCS#1 | `BEGIN RSA PRIVATE KEY` |
| `service-account-pkcs8.pem` | PKCS#8 | `BEGIN PRIVATE KEY` |

They were generated locally with `openssl genrsa` / `openssl genpkey`, have
never been uploaded anywhere, and authenticate nothing. A repository secret
scanner will still match on the PEM headers — allowlist this directory rather
than obfuscating the fixtures, which would only hide them from the next reader
too.

The alternative — generating a key inside the test — is not available: the crate
depends on `jsonwebtoken`, which can consume an RSA key but not create one.
