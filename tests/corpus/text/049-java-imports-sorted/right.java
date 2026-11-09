package sh.example.billing;

import java.io.IOException;
import java.time.Instant;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;

public final class InvoiceStore {
    private final Map<String, Invoice> byId = new HashMap<>();
}
