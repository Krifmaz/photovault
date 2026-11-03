module github.com/ente/cli/photovaultvectors

go 1.26.0

require (
	github.com/ente/cli v0.0.0
	github.com/ente/go-srp v0.0.0-20260624111502-867e68cac3e3
	github.com/kong/go-srp v0.0.0-20191210190804-cde1efa3c083
)

require (
	github.com/minio/blake2b-simd v0.0.0-20160723061019-3f5f724cb5b1 // indirect
	golang.org/x/crypto v0.57.0 // indirect
	golang.org/x/sys v0.48.0 // indirect
)

// ./ente-cli must point at the cli/ directory of an ente-io/ente checkout
// (a symlink is fine); see ../README.md.
replace github.com/ente/cli => ./ente-cli
