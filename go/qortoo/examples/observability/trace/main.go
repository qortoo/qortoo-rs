// Command trace demonstrates a Go OpenTelemetry span continued by Qortoo's Rust core.
//
// Run from go/qortoo:
//
//	go run ./examples/observability/trace
package main

import (
	"context"
	"errors"
	"fmt"
	"log"
	"os"
	"time"

	"github.com/qortoo/qortoo-rs/go/qortoo"
	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracegrpc"
	"go.opentelemetry.io/otel/sdk/resource"
	sdktrace "go.opentelemetry.io/otel/sdk/trace"
)

const (
	serviceName     = "qortoo-go-example-trace"
	defaultEndpoint = "http://localhost:4317"
)

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() error {
	ctx := context.Background()
	endpoint := traceEndpoint()

	tracerProvider, err := setupGoTracing(ctx, endpoint)
	if err != nil {
		return err
	}

	if err := qortoo.InitObservability(qortoo.ObservabilityOptions{
		ServiceName:  serviceName,
		LogFilter:    "qortoo=trace,qortoo_ffi=trace",
		LogFormat:    qortoo.LogFormatText,
		TraceEnabled: true,
		OTLPEndpoint: endpoint,
	}); err != nil {
		_ = tracerProvider.Shutdown(ctx)
		return fmt.Errorf("initialize Qortoo observability: %w", err)
	}

	fmt.Printf("Exporting traces -> %s\n", endpoint)
	fmt.Println("View in Grafana  -> http://localhost:3000 -> Explore -> Tempo")

	workloadErr := runCounterSync(ctx)
	shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	qortooErr := qortoo.ShutdownObservability(shutdownCtx)
	goErr := tracerProvider.Shutdown(shutdownCtx)

	return errors.Join(workloadErr, qortooErr, goErr)
}

func setupGoTracing(ctx context.Context, endpoint string) (*sdktrace.TracerProvider, error) {
	exporter, err := otlptracegrpc.New(ctx, otlptracegrpc.WithEndpointURL(endpoint))
	if err != nil {
		return nil, fmt.Errorf("create Go OTLP exporter: %w", err)
	}

	provider := sdktrace.NewTracerProvider(
		sdktrace.WithBatcher(exporter),
		sdktrace.WithResource(resource.NewSchemaless(
			attribute.String("service.name", serviceName),
			attribute.String("qortoo.binding.language", "go"),
		)),
	)
	otel.SetTracerProvider(provider)
	return provider, nil
}

func runCounterSync(ctx context.Context) error {
	ctx, span := otel.Tracer("qortoo-go-example").Start(ctx, "example.counter_sync")
	defer span.End()
	span.SetAttributes(
		attribute.String("collection", "example-trace"),
		attribute.Int("clients", 2),
	)

	connectivity := qortoo.NewLocalConnectivity()
	defer connectivity.Close()
	connectivity.SetRealtime(false)

	clientA, err := qortoo.NewClient(
		"example-trace",
		"client-a",
		qortoo.WithLocalConnectivity(connectivity),
	)
	if err != nil {
		return fmt.Errorf("create client-a: %w", err)
	}
	defer clientA.Close()

	clientB, err := qortoo.NewClient(
		"example-trace",
		"client-b",
		qortoo.WithLocalConnectivity(connectivity),
	)
	if err != nil {
		return fmt.Errorf("create client-b: %w", err)
	}
	defer clientB.Close()

	counterA, err := clientA.CreateCounter("shared-counter", nil)
	if err != nil {
		return fmt.Errorf("create counter: %w", err)
	}
	defer counterA.Close()

	if _, err := counterA.IncreaseBy(5); err != nil {
		return fmt.Errorf("increase counter: %w", err)
	}
	if err := counterA.SyncContext(ctx); err != nil {
		return fmt.Errorf("sync client-a: %w", err)
	}

	counterB, err := clientB.SubscribeCounter("shared-counter", nil)
	if err != nil {
		return fmt.Errorf("subscribe counter: %w", err)
	}
	defer counterB.Close()

	if err := counterB.SyncContext(ctx); err != nil {
		return fmt.Errorf("sync client-b: %w", err)
	}

	fmt.Printf(
		"counter values after sync: client-a=%d, client-b=%d\n",
		counterA.Value(),
		counterB.Value(),
	)
	return nil
}

func traceEndpoint() string {
	if endpoint := os.Getenv("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"); endpoint != "" {
		return endpoint
	}
	if endpoint := os.Getenv("OTEL_EXPORTER_OTLP_ENDPOINT"); endpoint != "" {
		return endpoint
	}
	return defaultEndpoint
}
