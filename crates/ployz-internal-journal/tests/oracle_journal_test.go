package journal

import (
	"bytes"
	"context"
	"encoding/hex"
	"fmt"
	"io"
	"os/exec"
	"testing"

	"github.com/psviderski/uncloud/pkg/api"
)

func TestPloyzJournalOracle(t *testing.T) {
	var output bytes.Buffer
	normal := []byte("1769188773.687500 first\n-- Boot marker --\r\n\n0.000000 \xff\xfe\nfinal\r")
	writeFollowOutput(t, &output, "normal", normal)
	writeFollowOutput(t, &output, "long", bytes.Repeat([]byte{'x'}, 64*1024))

	originalCommandContext := commandContext
	t.Cleanup(func() { commandContext = originalCommandContext })
	var commandName string
	var commandArgs []string
	commandContext = func(ctx context.Context, name string, args ...string) *exec.Cmd {
		commandName = name
		commandArgs = append([]string(nil), args...)
		return exec.CommandContext(ctx, "/bin/true")
	}
	reader, wait, err := logs(context.Background(), "uncloud.service", api.ServiceLogsOptions{
		Follow: true,
		Tail:   -1,
		Since:  "10 minutes ago",
		Until:  "now",
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err = io.ReadAll(reader); err != nil {
		t.Fatal(err)
	}
	if err = wait(); err != nil {
		t.Fatal(err)
	}
	fmt.Fprintf(&output, "command|%s\n", hex.EncodeToString([]byte(commandName)))
	for _, arg := range commandArgs {
		fmt.Fprintf(&output, "arg|%s\n", hex.EncodeToString([]byte(arg)))
	}

	fmt.Printf("PLOYZ_ORACLE_BEGIN\n%s\nPLOYZ_ORACLE_END\n", hex.EncodeToString(output.Bytes()))
}

func writeFollowOutput(t *testing.T, output *bytes.Buffer, label string, data []byte) {
	t.Helper()
	entries := make(chan api.LogEntry, 16)
	follow(context.Background(), bytes.NewReader(data), entries)
	close(entries)
	for logEntry := range entries {
		timestamp := "-"
		if !logEntry.Timestamp.IsZero() {
			timestamp = fmt.Sprintf("%d.%09d", logEntry.Timestamp.Unix(), logEntry.Timestamp.Nanosecond())
		}
		errText := ""
		if logEntry.Err != nil {
			errText = logEntry.Err.Error()
		}
		fmt.Fprintf(output, "%s|%d|%s|%s|%s\n",
			label,
			logEntry.Stream,
			timestamp,
			hex.EncodeToString(logEntry.Message),
			hex.EncodeToString([]byte(errText)),
		)
	}
}
