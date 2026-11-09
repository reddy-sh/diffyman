package server

import (
  "encoding/json"
  "net/http"
)

func Routes(mux *http.ServeMux) {
  mux.HandleFunc("/health", handleHealth)
  mux.HandleFunc("/orders", handleOrders)
}

func handleHealth(w http.ResponseWriter, r *http.Request) {
  w.Header().Set("Content-Type", "application/json")
  json.NewEncoder(w).Encode(map[string]string{"status": "ok"})
}

func handleOrders(w http.ResponseWriter, r *http.Request) {
  if r.Method != http.MethodGet {
    http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
    return
  }
  orders, err := store.ListOrders(r.Context())
  if err != nil {
    http.Error(w, err.Error(), http.StatusInternalServerError)
    return
  }
  json.NewEncoder(w).Encode(orders)
}
