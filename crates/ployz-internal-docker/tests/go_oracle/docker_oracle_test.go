package docker

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"
	"testing"

	"github.com/distribution/reference"
	"github.com/docker/docker/pkg/jsonmessage"
)

func TestBareHexIdentifierIsNotANormalizedImageReference(t *testing.T) {
	if _, err := reference.ParseNormalizedNamed(strings.Repeat("f", 64)); err == nil {
		t.Fatal("frozen Docker reference parser accepted a bare 64-hex identifier")
	}
	if _, err := reference.ParseNormalizedNamed("repo@sha256:" + strings.Repeat("f", 64)); err != nil {
		t.Fatalf("frozen Docker reference parser rejected a qualified digest: %v", err)
	}
}

func TestProgressFixtureMatchesRust(t *testing.T) {
	raw, err := os.Open("testdata/progress.stream.json")
	if err != nil {
		t.Fatal(err)
	}
	defer raw.Close()

	var output bytes.Buffer
	decoder := json.NewDecoder(raw)
	for {
		var message jsonmessage.JSONMessage
		if err := decoder.Decode(&message); err != nil {
			if err == io.EOF {
				break
			}
			t.Fatal(err)
		}
		var current, total, start int64
		var hideCounts bool
		var units string
		if message.Progress != nil {
			current = message.Progress.Current
			total = message.Progress.Total
			start = message.Progress.Start
			hideCounts = message.Progress.HideCounts
			units = message.Progress.Units
		}
		errorCode, errorMessage := 0, ""
		if message.Error != nil {
			errorCode = message.Error.Code
			errorMessage = message.Error.Message
		}
		aux := ""
		if message.Aux != nil {
			aux = string(*message.Aux)
		}
		fmt.Fprintf(&output, "%s\t%s\t%s\t%d\t%d\t%d\t%t\t%s\t%d\t%s\t%s\t%s\t%s\t%d\t%d\t%s\n",
			message.ID, message.Status, message.ProgressMessage, current, total, start,
			hideCounts, units, errorCode, errorMessage, message.ErrorMessage,
			message.Stream, message.From, message.Time, message.TimeNano, aux)
	}

	rustPath := os.Getenv("PLOYZ_RUST_PROGRESS_OUT")
	if rustPath == "" {
		t.Fatal("PLOYZ_RUST_PROGRESS_OUT is required")
	}
	rust, err := os.ReadFile(rustPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(output.Bytes(), rust) {
		t.Fatalf("Go/Rust progress mismatch\nGo:\n%s\nRust:\n%s", output.Bytes(), rust)
	}
}
