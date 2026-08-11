package proxy

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"syscall"
	"testing"
	"time"
)

func TestPloyzProxyOracle(t *testing.T) {
	tests := []struct {
		name string
		err  error
	}{
		{name: "net_closed", err: net.ErrClosed},
		{name: "closed_pipe", err: io.ErrClosedPipe},
		{name: "epipe", err: syscall.EPIPE},
		{name: "reset", err: syscall.ECONNRESET},
		{name: "wrapped_epipe", err: fmt.Errorf("copy data: %w", syscall.EPIPE)},
		{name: "other", err: errors.New("copy failed")},
	}

	for _, test := range tests {
		fmt.Printf("PLOYZ_ORACLE_CLOSED_%s=%t\n", test.name, IsConnectionClosedError(test.err))
	}

	listenerErr := errors.New("listener failed")
	err := (&Proxy{Listener: errorListener{err: listenerErr}}).Run(t.Context())
	fmt.Printf("PLOYZ_ORACLE_ACCEPT_ERROR=%s\n", err)

	remote, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer remote.Close()
	remoteDone := make(chan struct{})
	go func() {
		defer close(remoteDone)
		conn, acceptErr := remote.Accept()
		if acceptErr != nil {
			t.Error(acceptErr)
			return
		}
		defer conn.Close()
		request, readErr := io.ReadAll(conn)
		if readErr != nil {
			t.Error(readErr)
			return
		}
		if string(request) != "request" {
			t.Errorf("request = %q", request)
			return
		}
		if _, writeErr := conn.Write([]byte("response")); writeErr != nil {
			t.Error(writeErr)
			return
		}
		if halfCloser, ok := conn.(interface{ CloseWrite() error }); ok {
			_ = halfCloser.CloseWrite()
		}
	}()

	local, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	runDone := make(chan error, 1)
	go func() {
		runDone <- (&Proxy{Listener: local, RemoteAddr: remote.Addr().String()}).Run(ctx)
	}()
	client, err := net.Dial("tcp", local.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	if _, err = client.Write([]byte("request")); err != nil {
		t.Fatal(err)
	}
	if halfCloser, ok := client.(interface{ CloseWrite() error }); ok {
		if err = halfCloser.CloseWrite(); err != nil {
			t.Fatal(err)
		}
	}
	response, err := io.ReadAll(client)
	if err != nil {
		t.Fatal(err)
	}
	_ = client.Close()
	<-remoteDone
	fmt.Printf("PLOYZ_ORACLE_BIDIRECTIONAL=request->%s\n", response)
	cancel()
	select {
	case err = <-runDone:
		fmt.Printf("PLOYZ_ORACLE_CANCELLATION_NIL=%t\n", err == nil)
	case <-time.After(time.Second):
		t.Fatal("proxy cancellation timed out")
	}

	if runtime.GOOS != "windows" {
		socketPath := filepath.Join(t.TempDir(), "proxy.sock")
		unixListener, listenErr := net.Listen("unix", socketPath)
		if listenErr != nil {
			t.Fatal(listenErr)
		}
		unixCtx, cancelUnix := context.WithCancel(t.Context())
		unixDone := make(chan error, 1)
		go func() {
			unixDone <- (&Proxy{Listener: unixListener}).Run(unixCtx)
		}()
		cancelUnix()
		select {
		case err = <-unixDone:
			if err != nil {
				t.Fatal(err)
			}
		case <-time.After(time.Second):
			t.Fatal("Unix proxy cancellation timed out")
		}
		_, statErr := os.Stat(socketPath)
		fmt.Printf("PLOYZ_ORACLE_UNIX_UNLINKED=%t\n", errors.Is(statErr, os.ErrNotExist))
	}
}
