// Command metrics demonstrates Qortoo metrics exported to Prometheus.
//
// Run from go/qortoo:
//
//	go run ./examples/observability/metrics
package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/qortoo/qortoo-rs/go/qortoo"
)

const defaultListenAddress = "0.0.0.0:9000"

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() error {
	listenAddress := os.Getenv("QORTOO_METRICS_LISTEN_ADDR")
	if listenAddress == "" {
		listenAddress = defaultListenAddress
	}

	if err := qortoo.InitObservability(qortoo.ObservabilityOptions{
		ServiceName:       "qortoo-go-example-metrics",
		MetricsEnabled:    true,
		MetricsListenAddr: listenAddress,
	}); err != nil {
		return fmt.Errorf("initialize observability: %w", err)
	}
	defer func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := qortoo.ShutdownObservability(ctx); err != nil {
			log.Printf("shut down observability: %v", err)
		}
	}()

	client, err := qortoo.NewClient("example-metrics", "client-a")
	if err != nil {
		return fmt.Errorf("create client: %w", err)
	}
	defer client.Close()

	counter, err := client.CreateCounter("counter", nil)
	if err != nil {
		return fmt.Errorf("create counter: %w", err)
	}
	defer counter.Close()

	fmt.Printf("Scrape endpoint : http://%s/metrics\n", displayAddress(listenAddress))
	fmt.Println("Prometheus target: host.docker.internal:9000 (macOS/Windows)")
	fmt.Println("                   172.17.0.1:9000           (Linux / docker bridge)")
	fmt.Println("Ctrl-C to stop.")

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	ticker := time.NewTicker(5 * time.Second)
	defer ticker.Stop()

	var iteration int64
	for {
		iteration++
		value, err := counter.IncreaseBy(iteration)
		if err != nil {
			return fmt.Errorf("increase counter: %w", err)
		}
		if err := counter.Sync(); err != nil {
			return fmt.Errorf("sync counter: %w", err)
		}
		fmt.Printf("iteration %4d: counter = %d\n", iteration, value)

		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
		}
	}
}

func displayAddress(address string) string {
	if address == "0.0.0.0:9000" {
		return "localhost:9000"
	}
	return address
}
