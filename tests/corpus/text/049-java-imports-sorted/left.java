package sh.example.billing;

import java.util.Map;
import java.time.Instant;
import java.util.List;
import java.io.IOException;
import java.util.HashMap;

public final class InvoiceStore {
    private final Map<String, Invoice> byId = new HashMap<>();
}
