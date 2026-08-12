package proxy

import (
	"encoding/hex"
	"os"
	"strings"
	"testing"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func TestPloyzProxyFixtures(t *testing.T) {
	out := os.Getenv("PLOYZ_GO_FIXTURES_OUT")
	if out == "" {
		t.Fatal("PLOYZ_GO_FIXTURES_OUT is required")
	}

	backend := &MetadataBackend{
		MachineID:   "id-2",
		MachineName: "machine-b",
		MachineAddr: "fd00::2",
	}
	errorValue := status.Error(codes.PermissionDenied, "denied")
	type fixture struct {
		name string
		run  func() ([]byte, error)
	}
	fixtures := []fixture{
		{"append_streaming", func() ([]byte, error) { return backend.AppendInfo(true, []byte{0x08, 0x01}) }},
		{"append_unary", func() ([]byte, error) { return backend.AppendInfo(false, []byte{0x0a, 0x00}) }},
		{"build_error_streaming", func() ([]byte, error) { return backend.BuildError(true, errorValue) }},
		{"build_error_unary", func() ([]byte, error) { return backend.BuildError(false, errorValue) }},
	}

	var output strings.Builder
	for _, item := range fixtures {
		payload, err := item.run()
		if err != nil {
			t.Fatalf("%s: %v", item.name, err)
		}
		output.WriteString(item.name)
		output.WriteByte('\t')
		output.WriteString(hex.EncodeToString(payload))
		output.WriteByte('\n')
	}
	if err := os.WriteFile(out, []byte(output.String()), 0o600); err != nil {
		t.Fatal(err)
	}
}
