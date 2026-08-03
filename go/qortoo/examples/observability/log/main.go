// Command log demonstrates Qortoo's managed stdout logging from Go.
//
// Run from go/qortoo:
//
//	go run ./examples/observability/log
//	QORTOO_LOG_FORMAT=json RUST_LOG=qortoo=trace go run ./examples/observability/log
package main

import (
	"context"
	"fmt"
	"log"
	"os"

	"github.com/qortoo/qortoo-rs/go/qortoo"
)

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() error {
	format := qortoo.LogFormatText
	if os.Getenv("QORTOO_LOG_FORMAT") == "json" {
		format = qortoo.LogFormatJSON
	}
	filter := os.Getenv("RUST_LOG")
	if filter == "" {
		filter = "qortoo=trace"
	}

	if err := qortoo.InitObservability(qortoo.ObservabilityOptions{
		ServiceName: "qortoo-go-example-log",
		LogFilter:   filter,
		LogFormat:   format,
	}); err != nil {
		return fmt.Errorf("initialize observability: %w", err)
	}
	defer func() {
		if err := qortoo.ShutdownObservability(context.Background()); err != nil {
			log.Printf("shut down observability: %v", err)
		}
	}()

	connectivity := qortoo.NewLocalConnectivity()
	defer connectivity.Close()
	connectivity.SetRealtime(false)

	client, err := qortoo.NewClient(
		"example-log",
		"client-a",
		qortoo.WithLocalConnectivity(connectivity),
	)
	if err != nil {
		return fmt.Errorf("create client: %w", err)
	}
	defer client.Close()

	counter, err := client.CreateCounter("log-counter", nil)
	if err != nil {
		return fmt.Errorf("create counter: %w", err)
	}
	defer counter.Close()

	if _, err := counter.IncreaseBy(3); err != nil {
		return fmt.Errorf("increase counter: %w", err)
	}
	if err := counter.Sync(); err != nil {
		return fmt.Errorf("sync counter: %w", err)
	}

	fmt.Printf("counter value: %d\n", counter.Value())
	return nil
}
