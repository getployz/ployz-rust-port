package logs

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"os"
	"testing"
	"time"

	"github.com/psviderski/uncloud/pkg/api"
)

func TestFormatterFixturesMatchRust(t *testing.T) {
	timestamp := time.Unix(1735734645, 987654321)
	single := NewFormatter([]string{"worker-long", "worker-1"}, []string{"web"}, false)
	entries := []struct {
		formatter *Formatter
		entry     api.ServiceLogEntry
	}{
		{single, fixtureEntry(api.LogStreamStdout, timestamp, []byte("hello\xff\n"))},
		{single, fixtureEntry(api.LogStreamStderr, timestamp, []byte("bad\n"))},
		{single, fixtureEntry(api.LogStreamHeartbeat, timestamp, []byte("ignored"))},
	}

	multi := NewFormatter([]string{"long", "a"}, []string{"web", "api"}, false)
	multiEntry := fixtureEntry(api.LogStreamStdout, timestamp, []byte("multi\n"))
	multiEntry.Metadata.MachineName = "a"
	entries = append(entries, struct {
		formatter *Formatter
		entry     api.ServiceLogEntry
	}{multi, multiEntry})

	system := fixtureEntry(api.LogStreamStdout, timestamp, []byte("journal\n"))
	system.Metadata.MachineName = "a"
	system.Metadata.ServiceName = "api"
	system.Metadata.ContainerID = ""
	entries = append(entries, struct {
		formatter *Formatter
		entry     api.ServiceLogEntry
	}{multi, system})

	hooked := multiEntry
	hooked.Metadata.Hook = "pre-deploy"
	entries = append(entries, struct {
		formatter *Formatter
		entry     api.ServiceLogEntry
	}{multi, hooked})

	globalError := api.ServiceLogEntry{LogEntry: api.LogEntry{Err: errors.New("cluster disconnected")}}
	entries = append(entries, struct {
		formatter *Formatter
		entry     api.ServiceLogEntry
	}{multi, globalError})

	stalled := fixtureEntry(api.LogStreamUnknown, time.Time{}, nil)
	stalled.Err = fmt.Errorf("outer context: %w", api.ErrLogStreamStalled)
	entries = append(entries, struct {
		formatter *Formatter
		entry     api.ServiceLogEntry
	}{multi, stalled})

	systemError := system
	systemError.Err = errors.New("socket closed")
	entries = append(entries, struct {
		formatter *Formatter
		entry     api.ServiceLogEntry
	}{multi, systemError})

	var fixtures bytes.Buffer
	for _, fixture := range entries {
		stdout, stderr := captureEntry(t, fixture.formatter, fixture.entry)
		// Exact Lip Gloss/iocraft SGR encoding is an approved terminal-stack
		// limitation. Compare visible bytes and routing across implementations.
		fmt.Fprintf(&fixtures, "%x\t%x\n", stripSGR(stdout), stripSGR(stderr))
	}

	rustPath := os.Getenv("PLOYZ_RUST_LOG_FIXTURES_IN")
	if rustPath == "" {
		t.Skip("PLOYZ_RUST_LOG_FIXTURES_IN is not set")
	}
	rust, err := os.ReadFile(rustPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(fixtures.Bytes(), rust) {
		t.Fatalf("Go/Rust formatter mismatch\nGo:\n%s\nRust:\n%s", fixtures.Bytes(), rust)
	}
}

func stripSGR(input []byte) []byte {
	output := make([]byte, 0, len(input))
	for index := 0; index < len(input); {
		if input[index] == '\x1b' && index+1 < len(input) && input[index+1] == '[' {
			index += 2
			for index < len(input) {
				finalByte := input[index]
				index++
				if finalByte >= '@' && finalByte <= '~' {
					break
				}
			}
			continue
		}
		output = append(output, input[index])
		index++
	}
	return output
}

func fixtureEntry(stream api.LogStreamType, timestamp time.Time, message []byte) api.ServiceLogEntry {
	return api.ServiceLogEntry{
		Metadata: api.ServiceLogEntryMetadata{
			ServiceName: "web",
			ContainerID: "0123456789abcdef",
			MachineName: "worker-1",
		},
		LogEntry: api.LogEntry{Stream: stream, Timestamp: timestamp, Message: message},
	}
}

func captureEntry(t *testing.T, formatter *Formatter, entry api.ServiceLogEntry) ([]byte, []byte) {
	t.Helper()
	stdoutReader, stdoutWriter, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	stderrReader, stderrWriter, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}

	oldStdout, oldStderr := os.Stdout, os.Stderr
	os.Stdout, os.Stderr = stdoutWriter, stderrWriter
	formatter.PrintEntry(entry)
	os.Stdout, os.Stderr = oldStdout, oldStderr
	if err := stdoutWriter.Close(); err != nil {
		t.Fatal(err)
	}
	if err := stderrWriter.Close(); err != nil {
		t.Fatal(err)
	}
	stdout, err := io.ReadAll(stdoutReader)
	if err != nil {
		t.Fatal(err)
	}
	stderr, err := io.ReadAll(stderrReader)
	if err != nil {
		t.Fatal(err)
	}
	if err := stdoutReader.Close(); err != nil {
		t.Fatal(err)
	}
	if err := stderrReader.Close(); err != nil {
		t.Fatal(err)
	}
	return stdout, stderr
}
