package main

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"net/http"
	"os"
)

func main() {
	if len(os.Args) != 3 {
		panic("usage: tls_server ADDRESS CERTIFICATE_DIRECTORY")
	}
	address, certificates := os.Args[1], os.Args[2]
	caPEM, err := os.ReadFile(certificates + "/ca.pem")
	if err != nil {
		panic(err)
	}
	clientCAs := x509.NewCertPool()
	if !clientCAs.AppendCertsFromPEM(caPEM) {
		panic("parse client CA")
	}

	handler := http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		if request.Method != http.MethodPost || request.URL.Path != "/v1.53/images/create" {
			http.Error(response, "unexpected request", http.StatusBadRequest)
			return
		}
		query := request.URL.Query()
		if query.Get("fromImage") != "docker.io/library/busybox" || query.Get("tag") != "latest" {
			http.Error(response, "unexpected image reference", http.StatusBadRequest)
			return
		}
		if request.Header.Get("X-Registry-Auth") != "acceptance-token" {
			http.Error(response, "unexpected registry auth", http.StatusBadRequest)
			return
		}
		response.Header().Set("Content-Type", "application/json")
		fmt.Fprintln(response, `{"status":"tls-ok"}`)
	})
	server := &http.Server{
		Addr:    address,
		Handler: handler,
		TLSConfig: &tls.Config{
			MinVersion: tls.VersionTLS12,
			ClientAuth: tls.RequireAndVerifyClientCert,
			ClientCAs:  clientCAs,
		},
	}
	if err := server.ListenAndServeTLS(certificates+"/server.pem", certificates+"/server.key"); err != nil {
		panic(err)
	}
}
