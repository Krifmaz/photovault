// Command vectorgen produces (and checks) crypto test vectors for
// photovault-import using Ente's own Go code:
//
//   - github.com/ente/cli/internal/crypto: the Ente CLI's Argon2id key
//     derivation, login sub-key, secretstream, secretbox and sealed box.
//   - github.com/ente/go-srp: the SRP package Ente's server (museum) uses.
//   - github.com/kong/go-srp: the SRP client package the Ente CLI uses.
//
// Usage (see ../README.md):
//
//	go run . gen <out.json>       write go.json vectors
//	go run . verify <in.json>     decrypt Rust-produced ciphertexts with Ente's code
//	go run . srp-server           act as museum's SRP server on stdin/stdout
//
// This package lives under the github.com/ente/cli module path only so that
// Go's internal-package rule lets it import the CLI's internal/crypto.
package main

import (
	"bufio"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/big"
	"os"

	cli "github.com/ente/cli/internal/crypto"
	srp "github.com/ente/go-srp"
	kongsrp "github.com/kong/go-srp"
)

func b64(b []byte) string { return base64.StdEncoding.EncodeToString(b) }

func unb64(s string) []byte {
	b, err := base64.StdEncoding.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}

// pattern is the deterministic plaintext shared with the Node generator and
// the Rust tests: byte i = high byte of (i + seed) * 2654435761 (mod 2^32).
func pattern(seed uint32, n int) []byte {
	out := make([]byte, n)
	for i := range out {
		out[i] = byte(((uint32(i) + seed) * 2654435761) >> 24)
	}
	return out
}

// fixed returns n deterministic bytes derived from a label (SHA-256 counter mode).
func fixed(label string, n int) []byte {
	var out []byte
	for ctr := 0; len(out) < n; ctr++ {
		h := sha256.Sum256([]byte(fmt.Sprintf("%s/%d", label, ctr)))
		out = append(out, h[:]...)
	}
	return out[:n]
}

type argonVec struct {
	Password string `json:"password"`
	Salt     string `json:"salt"`
	OpsLimit int    `json:"ops_limit"`
	MemLimit int    `json:"mem_limit"`
	Key      string `json:"key"`
	LoginKey string `json:"login_key"`
}

type loginVec struct {
	Kek      string `json:"kek"`
	LoginKey string `json:"login_key"`
}

type blobVec struct {
	Key        string `json:"key"`
	Header     string `json:"header"`
	Plaintext  string `json:"plaintext"`
	Ciphertext string `json:"ciphertext"`
}

type srpVec struct {
	Note         string `json:"note"`
	Identity     string `json:"identity"`
	Salt         string `json:"salt"`
	LoginKey     string `json:"login_key"`
	ClientSecret string `json:"client_secret"`
	ServerSecret string `json:"server_secret"`
	Verifier     string `json:"verifier"`
	A            string `json:"a_pub"`
	B            string `json:"b_pub"`
	M1           string `json:"m1"`
	M2           string `json:"m2"`
	K            string `json:"k"`
	KongM1       string `json:"kong_cli_m1"`
}

type goVectors struct {
	Generator string     `json:"generator"`
	Argon     []argonVec `json:"argon"`
	Login     []loginVec `json:"login_key"`
	Blob      []blobVec  `json:"blob"`
	Srp       []srpVec   `json:"srp"`
}

func gen(path string) error {
	v := goVectors{Generator: "importers/tests/vectors/gen/go (Ente CLI internal/crypto, ente/go-srp, kong/go-srp)"}

	// Argon2id exactly as the CLI derives the key-encryption key.
	for _, a := range []struct {
		pw        string
		ops, mem  int
		saltLabel string
	}{
		{"correct horse battery staple", 2, 64 * 1024 * 1024, "argon-salt-1"}, // INTERACTIVE
		{"pässwörd ✓", 1, 8192, "argon-salt-2"},                               // libsodium minimums
		{"", 3, 16 * 1024 * 1024, "argon-salt-3"},
	} {
		salt := b64(fixed(a.saltLabel, 16))
		key, err := cli.DeriveArgonKey(a.pw, salt, a.mem, a.ops)
		if err != nil {
			return err
		}
		v.Argon = append(v.Argon, argonVec{a.pw, salt, a.ops, a.mem, b64(key), b64(cli.DeriveLoginKey(key))})
	}

	for i := 0; i < 3; i++ {
		kek := fixed(fmt.Sprintf("kek-%d", i), 32)
		v.Login = append(v.Login, loginVec{b64(kek), b64(cli.DeriveLoginKey(kek))})
	}

	// Single FINAL-tagged secretstream messages (metadata / thumbnails).
	for i, n := range []int{0, 1, 33, 1000} {
		key := fixed(fmt.Sprintf("blob-key-%d", i), 32)
		pt := pattern(uint32(i), n)
		ct, header, err := cli.EncryptChaCha20poly1305(pt, key)
		if err != nil {
			return err
		}
		v.Blob = append(v.Blob, blobVec{b64(key), b64(header), b64(pt), b64(ct)})
	}

	// SRP: museum's server (ente/go-srp) against its client, fixed secrets.
	params := srp.GetParams(4096)
	nBytes := params.NLengthBits / 8
	addSrp := func(note, identity string, salt, loginKey, a, b []byte) {
		verifier := srp.ComputeVerifier(params, salt, []byte(identity), loginKey)
		client := srp.NewClient(params, salt, []byte(identity), loginKey, a)
		A := client.ComputeA()
		server := srp.NewServer(params, verifier, b)
		B := server.ComputeB()
		server.SetA(A)
		client.SetB(B)
		m1 := client.ComputeM1()
		m2, err := server.CheckM1(m1)
		if err != nil {
			panic(err)
		}
		if err := client.CheckM2(m2); err != nil {
			panic(err)
		}
		// The CLI's kong/go-srp client sends A unpadded and hashes it
		// unpadded; record its M1 for the same inputs for reference.
		kp := kongsrp.GetParams(4096)
		kc := kongsrp.NewClient(kp, salt, []byte(identity), loginKey, a)
		kc.SetB(B)
		v.Srp = append(v.Srp, srpVec{
			Note: note, Identity: identity, Salt: b64(salt), LoginKey: b64(loginKey),
			ClientSecret: b64(a), ServerSecret: b64(b), Verifier: b64(verifier),
			A: b64(A), B: b64(B), M1: b64(m1), M2: b64(m2), K: b64(server.ComputeK()),
			KongM1: b64(kc.ComputeM1()),
		})
	}
	ident := "8f1c2a7e-4b3d-4e6f-9a1b-2c3d4e5f6a7b"
	addSrp("plain", ident, fixed("srp-salt", 16), fixed("srp-login", 16), fixed("srp-a", 32), fixed("srp-b", 32))
	// Find secrets whose A, B and S have a leading zero byte to pin the
	// padding conventions (each happens with probability 1/256).
	found := map[string]bool{}
	for i := 0; len(found) < 3 && i < 100000; i++ {
		a := fixed(fmt.Sprintf("srp-a-%d", i), 32)
		b := fixed(fmt.Sprintf("srp-b-%d", i), 32)
		salt := fixed("srp-salt-z", 16)
		lk := fixed("srp-login-z", 16)
		verifier := srp.ComputeVerifier(params, salt, []byte(ident), lk)
		c := srp.NewClient(params, salt, []byte(ident), lk, a)
		A := c.ComputeA()
		s := srp.NewServer(params, verifier, b)
		B := s.ComputeB()
		if len(A) != nBytes || len(B) != nBytes {
			return errors.New("unexpected padding")
		}
		switch {
		case A[0] == 0 && !found["A"]:
			found["A"] = true
			addSrp("A has a leading zero byte", ident, salt, lk, a, b)
		case B[0] == 0 && !found["B"]:
			found["B"] = true
			addSrp("B has a leading zero byte", ident, salt, lk, a, b)
		case !found["S"]:
			// S is not exported; detect via K = H(pad(S)) vs H(strip(S)).
			s.SetA(A)
			if sLeadingZero(s, A) {
				found["S"] = true
				addSrp("S has a leading zero byte", ident, salt, lk, a, b)
			}
		}
	}

	out, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(out, '\n'), 0o644)
}

// sLeadingZero recomputes S = (A * v^u)^b mod N for a server that has seen
// A (go-srp keeps S private), checks it against the server's K = H(pad(S)),
// and reports whether its 512-byte encoding starts with a zero byte.
func sLeadingZero(s *srp.SRPServer, A []byte) bool {
	p := s.Params
	nLen := p.NLengthBits / 8
	pad := func(x *big.Int) []byte {
		b := x.Bytes()
		return append(make([]byte, nLen-len(b)), b...)
	}
	h := sha256.New()
	h.Write(A)
	h.Write(pad(s.B))
	u := new(big.Int).SetBytes(h.Sum(nil))
	vu := new(big.Int).Exp(s.Verifier, u, p.N)
	base := new(big.Int).Mul(new(big.Int).SetBytes(A), vu)
	S := new(big.Int).Exp(base, s.Secret2, p.N)
	k := sha256.Sum256(pad(S))
	if string(k[:]) != string(s.ComputeK()) {
		panic("S recomputation disagrees with go-srp")
	}
	return pad(S)[0] == 0
}

type verifyInput struct {
	Stream []struct {
		Key        string `json:"key"`
		Header     string `json:"header"`
		Seed       uint32 `json:"seed"`
		Len        int    `json:"len"`
		CipherFile string `json:"ciphertext_file"`
	} `json:"stream"`
	Blob      []blobVec `json:"blob"`
	Secretbox []struct {
		Key        string `json:"key"`
		Nonce      string `json:"nonce"`
		Plaintext  string `json:"plaintext"`
		Ciphertext string `json:"ciphertext"`
	} `json:"secretbox"`
	Sealed []struct {
		PublicKey  string `json:"public_key"`
		SecretKey  string `json:"secret_key"`
		Plaintext  string `json:"plaintext"`
		Ciphertext string `json:"ciphertext"`
	} `json:"sealed"`
	Argon []argonVec `json:"argon"`
}

// decryptEnteStream decrypts a file the way Ente's clients do: 4 MiB + 17
// byte ciphertext chunks, the last one tagged FINAL, nothing after it.
func decryptEnteStream(r io.Reader, key, header []byte) ([]byte, error) {
	d, err := cli.NewDecryptor(key, header)
	if err != nil {
		return nil, err
	}
	buf := make([]byte, 4*1024*1024+cli.XChaCha20Poly1305IetfABYTES)
	var out []byte
	for {
		n, err := io.ReadFull(r, buf)
		if n == 0 {
			return nil, errors.New("stream ended without a FINAL chunk")
		}
		if err != nil && !errors.Is(err, io.ErrUnexpectedEOF) {
			return nil, err
		}
		m, tag, perr := d.Pull(buf[:n])
		if perr != nil {
			return nil, perr
		}
		out = append(out, m...)
		if tag == cli.TagFinal {
			var one [1]byte
			if k, _ := r.Read(one[:]); k != 0 {
				return nil, errors.New("data after FINAL chunk")
			}
			return out, nil
		}
		if tag != cli.TagMessage || n != len(buf) {
			return nil, fmt.Errorf("bad non-final chunk: tag %d, %d bytes", tag, n)
		}
	}
}

func verify(path string) error {
	raw, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var in verifyInput
	if err := json.Unmarshal(raw, &in); err != nil {
		return err
	}
	checks := 0
	for _, s := range in.Stream {
		f, err := os.Open(s.CipherFile)
		if err != nil {
			return err
		}
		pt, err := decryptEnteStream(bufio.NewReader(f), unb64(s.Key), unb64(s.Header))
		f.Close()
		if err != nil {
			return fmt.Errorf("stream len %d: %w", s.Len, err)
		}
		if string(pt) != string(pattern(s.Seed, s.Len)) {
			return fmt.Errorf("stream len %d: plaintext mismatch", s.Len)
		}
		checks++
	}
	for _, b := range in.Blob {
		_, pt, err := cli.DecryptChaChaBase64(b.Ciphertext, unb64(b.Key), b.Header)
		if err != nil || string(pt) != string(unb64(b.Plaintext)) {
			return fmt.Errorf("blob: %v", err)
		}
		checks++
	}
	for _, s := range in.Secretbox {
		pt, err := cli.SecretBoxOpenBase64(s.Ciphertext, s.Nonce, unb64(s.Key))
		if err != nil || string(pt) != string(unb64(s.Plaintext)) {
			return fmt.Errorf("secretbox: %v", err)
		}
		checks++
	}
	for _, s := range in.Sealed {
		pt, err := cli.SealedBoxOpen(unb64(s.Ciphertext), unb64(s.PublicKey), unb64(s.SecretKey))
		if err != nil || string(pt) != string(unb64(s.Plaintext)) {
			return fmt.Errorf("sealed box: %v", err)
		}
		checks++
	}
	for _, a := range in.Argon {
		key, err := cli.DeriveArgonKey(a.Password, a.Salt, a.MemLimit, a.OpsLimit)
		if err != nil || b64(key) != a.Key || b64(cli.DeriveLoginKey(key)) != a.LoginKey {
			return fmt.Errorf("argon: %v", err)
		}
		checks++
	}
	fmt.Printf("OK %d checks\n", checks)
	return nil
}

// srpServer plays museum's SRP role over stdin/stdout, one JSON object per
// line: it reads {"verifier","a_pub"}, writes {"b_pub"}, reads {"m1"} and
// writes {"m2"} (or {"error"}). The server secret is random, as in museum.
func srpServer() error {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<20)
	out := json.NewEncoder(os.Stdout)
	var first struct {
		Verifier string `json:"verifier"`
		A        string `json:"a_pub"`
	}
	if !in.Scan() {
		return errors.New("no input")
	}
	if err := json.Unmarshal(in.Bytes(), &first); err != nil {
		return err
	}
	params := srp.GetParams(4096)
	server := srp.NewServer(params, unb64(first.Verifier), srp.GenKey())
	if err := out.Encode(map[string]string{"b_pub": b64(server.ComputeB())}); err != nil {
		return err
	}
	server.SetA(unb64(first.A))
	var second struct {
		M1 string `json:"m1"`
	}
	if !in.Scan() {
		return errors.New("no m1")
	}
	if err := json.Unmarshal(in.Bytes(), &second); err != nil {
		return err
	}
	m2, err := server.CheckM1(unb64(second.M1))
	if err != nil {
		return out.Encode(map[string]string{"error": err.Error()})
	}
	return out.Encode(map[string]string{"m2": b64(m2)})
}

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: vectorgen gen <out.json> | verify <in.json> | srp-server")
		os.Exit(2)
	}
	var err error
	switch os.Args[1] {
	case "gen":
		err = gen(os.Args[2])
	case "verify":
		err = verify(os.Args[2])
	case "srp-server":
		err = srpServer()
	default:
		err = fmt.Errorf("unknown command %q", os.Args[1])
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "error:", err)
		os.Exit(1)
	}
}
