package nativehooks

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"math"
	"os"
	"testing"
	"time"
)

type golden struct {
	Name        string `json:"name"`
	Hex         string `json:"hex"`
	FDCount     int    `json:"fd_count"`
	HeaderOK    bool   `json:"header_ok"`
	AppRequired *bool  `json:"app_required"`
	FeaturesOK  bool   `json:"features_ok"`
}

func vectors(t *testing.T) []golden {
	t.Helper()
	b, err := os.ReadFile("frozen-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var data struct {
		Contract string   `json:"contract"`
		Vectors  []golden `json:"vectors"`
	}
	if err := json.Unmarshal(b, &data); err != nil {
		t.Fatal(err)
	}
	if data.Contract != "YNKBROK1-v1" {
		t.Fatal(data.Contract)
	}
	return data.Vectors
}
func wireBytes(t *testing.T, g golden) []byte {
	t.Helper()
	b, err := hex.DecodeString(g.Hex)
	if err != nil {
		t.Fatal(err)
	}
	return b
}
func TestFrozenRustGolden(t *testing.T) {
	for _, g := range vectors(t) {
		t.Run(g.Name, func(t *testing.T) {
			b := wireBytes(t, g)
			f, err := DecodeFrame(b, g.FDCount)
			if (err == nil) != g.HeaderOK {
				t.Fatalf("decode=%v expected=%v", err, g.HeaderOK)
			}
			if g.AppRequired != nil {
				_, err := f.SelectedFeatures(*g.AppRequired, AllFeatures)
				if (err == nil) != g.FeaturesOK {
					t.Fatal("feature mismatch", err)
				}
			}
			if g.HeaderOK {
				binding := Binding{Nonce: f.Nonce, ConfigDigest: f.Header.Digest, Epoch: f.Header.Epoch}
				if f.CheckBinding(binding, f.Header.RequestID) != nil {
					t.Fatal("binding")
				}
				binding.Nonce[0] ^= 1
				if f.CheckBinding(binding, f.Header.RequestID) == nil {
					t.Fatal("nonce mismatch accepted")
				}
				for n := 0; n < len(b); n++ {
					if _, err := DecodeFrame(b[:n], g.FDCount); err == nil {
						t.Fatalf("prefix %d accepted", n)
					}
				}
				for _, count := range []int{-1, 0, 1, 2, 3} {
					if count != g.FDCount {
						if _, err := DecodeFrame(b, count); err == nil {
							t.Fatalf("FD count %d accepted", count)
						}
					}
				}
				var encoded []byte
				switch f.Header.Kind {
				case LookupKind:
					encoded, err = EncodeLookup(Binding{Nonce: f.Nonce, ConfigDigest: f.Header.Digest, Epoch: f.Header.Epoch}, f.Header.RequestID, f.Lookup)
				case FeaturesKind:
					encoded, err = EncodeFeatures(Binding{Nonce: f.Nonce, ConfigDigest: f.Header.Digest, Epoch: f.Header.Epoch}, f.Selected, false, AllFeatures)
				case AttachACKKind:
					encoded, err = EncodeAttachACK(Binding{Nonce: f.Nonce, ConfigDigest: f.Header.Digest, Epoch: f.Header.Epoch})
				}
				if encoded != nil && (err != nil || !bytes.Equal(encoded, b)) {
					t.Fatal("encoder differs from Rust golden", err)
				}
			}
		})
	}
}
func TestCookieNeverBecomesRequestFilter(t *testing.T) {
	for _, g := range vectors(t) {
		if g.Name == "known_observed_projection" {
			b := wireBytes(t, g)
			first, err := DecodeFrame(b, 0)
			if err != nil {
				t.Fatal(err)
			}
			be.PutUint64(b[112:], first.Reply.Cookie+1)
			second, err := DecodeFrame(b, 0)
			if err != nil || second.Reply.Cookie == first.Reply.Cookie {
				t.Fatal(err)
			}
			_, proof, _ := fixture(t)
			binding := Binding{Nonce: first.Nonce, Epoch: first.Header.Epoch, ConfigDigest: first.Header.Digest}
			request, err := EncodeLookup(binding, 1, proof.Flow)
			if err != nil {
				t.Fatal(err)
			}
			if !zero(request[98:100]) || !zero(request[136:144]) {
				t.Fatal("reply cookie leaked into request")
			}
		}
	}
}
func TestSelectedFeatureMatrix(t *testing.T) {
	_, proof, _ := fixture(t)
	for selected := uint64(0); selected < 16; selected++ {
		for _, app := range []bool{false, true} {
			_, err := EncodeFeatures(proof.Binding, selected, app, AllFeatures)
			want := selected < 8 && selected&OwnedTUN != 0 && (!app || selected == AllFeatures)
			if (err == nil) != want {
				t.Fatal(selected, app, err)
			}
		}
	}
	if _, err := EncodeFeatures(proof.Binding, AllFeatures, true, OwnedTUN); err == nil {
		t.Fatal("offer downgrade")
	}
}
func TestSequenceRetires(t *testing.T) {
	for _, failure := range []string{"outstanding", "reuse", "late", "wrap", "timeout"} {
		t.Run(failure, func(t *testing.T) {
			var s Sequence
			id := uint64(1)
			if failure == "wrap" {
				id = math.MaxUint64
			}
			if s.Begin(id) != nil {
				t.Fatal("begin")
			}
			switch failure {
			case "outstanding":
				if s.Begin(2) == nil {
					t.Fatal("second request")
				}
			case "late":
				if s.Complete(2) == nil {
					t.Fatal("late reply")
				}
			case "timeout":
				s.Retire()
			default:
				if s.Complete(id) != nil {
					t.Fatal("complete")
				}
				if s.Begin(1) == nil {
					t.Fatal("id reuse/wrap")
				}
			}
			if s.Begin(3) == nil || s.Complete(id) == nil {
				t.Fatal("retired endpoint reused")
			}
		})
	}
	var s Sequence
	if s.Complete(0) == nil {
		t.Fatal("unsolicited reply")
	}
}
func TestFullRTTTTL(t *testing.T) {
	now := time.Now()
	for _, c := range []struct{ ttl, rtt, want time.Duration }{{OwnerTTL, time.Millisecond, 19 * time.Millisecond}, {OwnerTTL, OwnerTTL, 0}, {OwnerTTL, OwnerTTL + 1, 0}, {OwnerTTL + 1, 0, 0}, {0, 0, 0}, {OwnerTTL, -1, 0}} {
		if got := EffectiveReplyTTL(c.ttl, now, now.Add(c.rtt)); got != c.want {
			t.Fatal(c, got)
		}
	}
}
func TestMalformedReservedPathAndBounds(t *testing.T) {
	for _, g := range vectors(t) {
		if !g.HeaderOK {
			continue
		}
		base := wireBytes(t, g)
		for _, n := range []int{769, 1024} {
			bad := make([]byte, n)
			copy(bad, base)
			be.PutUint32(bad[12:], uint32(n))
			if _, err := DecodeFrame(bad, g.FDCount); err == nil {
				t.Fatal("oversized")
			}
		}
		offsets := []int{8, 12, 24, 64}
		switch Kind(u16(base, 10)) {
		case HelloKind:
			offsets = append(offsets, 142)
		case AttachACKKind, RevokeKind:
			offsets = append(offsets, 100)
		case LookupKind:
			offsets = append(offsets, 98, 99, 136, 143)
		case ReplyKind:
			offsets = append(offsets, 98, 102, 124, 152)
		}
		for _, at := range offsets {
			bad := bytes.Clone(base)
			switch at {
			case 24:
				clear(bad[24:32])
			case 64:
				clear(bad[64:96])
			default:
				bad[at] ^= 1
			}
			if _, err := DecodeFrame(bad, g.FDCount); err == nil {
				t.Fatalf("%s offset %d", g.Name, at)
			}
		}
	}
	for _, p := range [][]byte{nil, []byte("relative"), []byte("/bad\x00path"), []byte("/bad (deleted)"), {'/', 255}, bytes.Repeat([]byte{'/'}, 513)} {
		if validPath(p) {
			t.Fatal("path accepted", p)
		}
	}
}

func TestLookupRejectsNoncanonicalAddresses(t *testing.T) {
	for _, g := range vectors(t) {
		b := wireBytes(t, g)
		if !g.HeaderOK || Kind(u16(b, 10)) != LookupKind {
			continue
		}
		clear(b[104:120])
		if _, err := DecodeFrame(b, 0); err == nil {
			t.Fatal("unspecified source accepted")
		}
	}
}

func FuzzDecodeFrame(f *testing.F) {
	f.Add([]byte("YNKBROK1"), 0)
	f.Add(make([]byte, MaxFrameBytes), 1)
	f.Fuzz(func(t *testing.T, b []byte, fds int) {
		frame, err := DecodeFrame(b, fds)
		if err == nil && frame.Header.Kind == LookupKind && !frame.Lookup.valid() {
			t.Fatal("неканонический tuple принят декодером")
		}
	})
}
