package log

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"log/slog"
	"math"
	"testing"
)

func TestPloyzLogOracle(t *testing.T) {
	var output bytes.Buffer
	handler := NewSlogTextHandler(&output, &slog.HandlerOptions{Level: slog.LevelDebug})
	logger := slog.New(handler).
		With("component", "dns server").
		WithGroup("request").
		With("name", "example.org.").
		WithGroup("details")

	logger.Debug("received", "kind", "A", "empty", "", "quoted", "a=b", "line", "a\nb",
		"ok", true, "signed", int64(-7), "unsigned", uint64(8), "small", 0.00001,
		"large", 1000000.0, "inf", math.Inf(1), "time", "removed", "level", "removed", "msg", "removed")
	logger.Info("no fields")
	logger.Warn("warning", "unicode", "hello-world", "combining", "x\u0301", "go_unassigned", "x\u088f", "zero_width", "a\u200bb")
	logger.Error("failure", "err", fmt.Errorf("bad value: %d", 3))
	slog.New(handler).WithGroup("").Info("root empty", "key", "value")
	slog.New(handler).WithGroup("parent").WithGroup("").Info("nested empty", "key", "value")

	fmt.Printf("PLOYZ_ORACLE_BEGIN\n%s\nPLOYZ_ORACLE_END\n", hex.EncodeToString(output.Bytes()))
}
