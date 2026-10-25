-- =====================================================================
-- BackPOS — Retail Store Point of Sale & Inventory Management System
-- Initial schema. Matches the DBMS project specification table set:
--   USER, CASHIER, INVENTORY_MANAGER, SUPPLIER, CUSTOMER, CATEGORY,
--   PRODUCT, STOCK_BATCH, PURCHASE_ORDER, SUPPLIER_DELIVERY, TRANSACTION,
--   TRANSACTION_ITEM, PAYMENT, STOCK_ADJUSTMENT, TAX_CONFIGURATION,
--   CUSTOMER_LOYALTY_LOG, PRODUCT_PRICE_HISTORY, NOTIFICATION
-- plus STORE for multi-location inventory.
-- Normalized to 3NF; InnoDB for FK integrity + row-level locking
-- (SELECT ... FOR UPDATE) used by the transaction service to prevent overselling.
-- =====================================================================

-- ---------------------------------------------------------------- multi-location
CREATE TABLE `STORE` (
    id          CHAR(36)     NOT NULL,
    code        VARCHAR(16)  NOT NULL,
    name        VARCHAR(120) NOT NULL,
    address     VARCHAR(255) NULL,
    phone       VARCHAR(32)  NULL,
    is_online   BOOLEAN      NOT NULL DEFAULT FALSE,
    created_at  DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_store PRIMARY KEY (id),
    CONSTRAINT uq_store_code UNIQUE (code)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- tax records
CREATE TABLE `TAX_CONFIGURATION` (
    id         CHAR(36)     NOT NULL,
    name       VARCHAR(60)  NOT NULL,
    rate       DECIMAL(5, 2) NOT NULL,
    is_default BOOLEAN      NOT NULL DEFAULT FALSE,
    is_active  BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_tax_configuration PRIMARY KEY (id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- suppliers
CREATE TABLE `SUPPLIER` (
    id          CHAR(36)     NOT NULL,
    name        VARCHAR(120) NOT NULL,
    contact_name VARCHAR(120) NULL,
    email       VARCHAR(160) NULL,
    phone       VARCHAR(32)  NULL,
    address     VARCHAR(255) NULL,
    is_active   BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at  DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_supplier PRIMARY KEY (id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- users (RBAC core)
-- Role subtypes: CASHIER / INVENTORY_MANAGER (1:1 specialisation tables below),
-- SUPPLIER users link to a vendor record via supplier_id.
CREATE TABLE `USER` (
    id             CHAR(36)      NOT NULL,
    name           VARCHAR(120)  NOT NULL,
    email          VARCHAR(160)  NOT NULL,
    phone          VARCHAR(32)   NULL,
    password_hash  VARCHAR(80)   NULL,
    pin_hash       VARCHAR(80)   NULL,
    role           ENUM ('ADMIN','MANAGER','INVENTORY_MANAGER','CASHIER','SUPPLIER') NOT NULL,
    account_status ENUM ('ACTIVE','SUSPENDED','PENDING') NOT NULL DEFAULT 'ACTIVE',
    store_id       CHAR(36)      NULL,
    supplier_id    CHAR(36)      NULL,
    created_at     DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    CONSTRAINT pk_user PRIMARY KEY (id),
    CONSTRAINT uq_user_email UNIQUE (email),
    CONSTRAINT fk_user_store    FOREIGN KEY (store_id)    REFERENCES `STORE` (id),
    CONSTRAINT fk_user_supplier FOREIGN KEY (supplier_id) REFERENCES `SUPPLIER` (id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- 1:1 specialisations of USER (enable role-specific columns + joins)
CREATE TABLE `CASHIER` (
    user_id       CHAR(36)    NOT NULL,
    employee_code VARCHAR(24) NOT NULL,
    hired_at      DATETIME    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_cashier PRIMARY KEY (user_id),
    CONSTRAINT uq_cashier_code UNIQUE (employee_code),
    CONSTRAINT fk_cashier_user FOREIGN KEY (user_id) REFERENCES `USER` (id) ON DELETE CASCADE
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

CREATE TABLE `INVENTORY_MANAGER` (
    user_id       CHAR(36)    NOT NULL,
    employee_code VARCHAR(24) NOT NULL,
    department    VARCHAR(60) NOT NULL DEFAULT 'Inventory',
    hired_at      DATETIME    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_inventory_manager PRIMARY KEY (user_id),
    CONSTRAINT uq_inventory_manager_code UNIQUE (employee_code),
    CONSTRAINT fk_inventory_manager_user FOREIGN KEY (user_id) REFERENCES `USER` (id) ON DELETE CASCADE
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- customers + loyalty
CREATE TABLE `CUSTOMER` (
    id             CHAR(36)     NOT NULL,
    name           VARCHAR(120) NOT NULL,
    phone          VARCHAR(32)  NULL,
    email          VARCHAR(160) NULL,
    loyalty_points INT          NOT NULL DEFAULT 0,
    is_member      BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at     DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_customer PRIMARY KEY (id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- catalog
CREATE TABLE `CATEGORY` (
    id          CHAR(36)     NOT NULL,
    name        VARCHAR(80)  NOT NULL,
    description VARCHAR(255) NULL,
    parent_id   CHAR(36)     NULL,
    CONSTRAINT pk_category PRIMARY KEY (id),
    CONSTRAINT uq_category_name UNIQUE (name),
    CONSTRAINT fk_category_parent FOREIGN KEY (parent_id) REFERENCES `CATEGORY` (id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

CREATE TABLE `PRODUCT` (
    id                CHAR(36)      NOT NULL,
    sku               VARCHAR(40)   NOT NULL,
    name              VARCHAR(160)  NOT NULL,
    barcode           VARCHAR(64)   NULL,
    barcode_type      ENUM ('UPC-A','EAN-13','EAN-8','CODE39','UNKNOWN') NOT NULL DEFAULT 'UNKNOWN',
    category_id       CHAR(36)      NULL,
    cost_price        DECIMAL(12,2) NOT NULL DEFAULT 0.00,
    selling_price     DECIMAL(12,2) NOT NULL DEFAULT 0.00,
    tax_rate_override DECIMAL(5,2)  NULL,
    unit              ENUM ('PIECE','KG','L','BOX','PACK') NOT NULL DEFAULT 'PIECE',
    reorder_threshold INT           NOT NULL DEFAULT 0,
    is_active         BOOLEAN       NOT NULL DEFAULT TRUE,
    created_at        DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at        DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    CONSTRAINT pk_product PRIMARY KEY (id),
    CONSTRAINT uq_product_sku UNIQUE (sku),
    CONSTRAINT uq_product_barcode UNIQUE (barcode),
    CONSTRAINT fk_product_category FOREIGN KEY (category_id) REFERENCES `CATEGORY` (id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- stock batches
-- One row per physical receipt lot of a product in a store; sale deduction
-- consumes batches FIFO (first received, first exhausted) under FOR UPDATE locks.
CREATE TABLE `STOCK_BATCH` (
    id          CHAR(36)      NOT NULL,
    product_id  CHAR(36)      NOT NULL,
    store_id    CHAR(36)      NOT NULL,
    quantity    INT           NOT NULL,
    unit_cost   DECIMAL(12,2) NOT NULL DEFAULT 0.00,
    batch_code  VARCHAR(40)   NULL,
    expires_at  DATE          NULL,
    received_at DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at  DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_stock_batch PRIMARY KEY (id),
    CONSTRAINT fk_stock_batch_product FOREIGN KEY (product_id) REFERENCES `PRODUCT` (id),
    CONSTRAINT fk_stock_batch_store   FOREIGN KEY (store_id)   REFERENCES `STORE` (id),
    CONSTRAINT idx_stock_batch_fifo INDEX (product_id, store_id, received_at)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- procurement
CREATE TABLE `PURCHASE_ORDER` (
    id          CHAR(36)      NOT NULL,
    po_number   VARCHAR(32)   NOT NULL,
    store_id    CHAR(36)      NOT NULL,
    supplier_id CHAR(36)      NOT NULL,
    created_by  CHAR(36)      NOT NULL,
    status      ENUM ('DRAFT','SUBMITTED','APPROVED','RECEIVED','CANCELLED') NOT NULL DEFAULT 'DRAFT',
    total       DECIMAL(14,2) NOT NULL DEFAULT 0.00,
    expected_at DATETIME      NULL,
    notes       VARCHAR(500)  NULL,
    created_at  DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    CONSTRAINT pk_purchase_order PRIMARY KEY (id),
    CONSTRAINT uq_purchase_order_number UNIQUE (po_number),
    CONSTRAINT fk_purchase_order_store    FOREIGN KEY (store_id)    REFERENCES `STORE` (id),
    CONSTRAINT fk_purchase_order_supplier FOREIGN KEY (supplier_id) REFERENCES `SUPPLIER` (id),
    CONSTRAINT fk_purchase_order_creator  FOREIGN KEY (created_by)  REFERENCES `USER` (id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

CREATE TABLE `PURCHASE_ORDER_ITEM` (
    id                CHAR(36)      NOT NULL,
    purchase_order_id CHAR(36)      NOT NULL,
    product_id        CHAR(36)      NOT NULL,
    quantity_ordered  INT           NOT NULL,
    quantity_received INT           NOT NULL DEFAULT 0,
    unit_cost         DECIMAL(12,2) NOT NULL DEFAULT 0.00,
    line_total        DECIMAL(14,2) NOT NULL DEFAULT 0.00,
    CONSTRAINT pk_purchase_order_item PRIMARY KEY (id),
    CONSTRAINT fk_poi_order   FOREIGN KEY (purchase_order_id) REFERENCES `PURCHASE_ORDER` (id) ON DELETE CASCADE,
    CONSTRAINT fk_poi_product FOREIGN KEY (product_id)        REFERENCES `PRODUCT` (id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- One-to-many: a PO can arrive in several partial deliveries.
CREATE TABLE `SUPPLIER_DELIVERY` (
    id                CHAR(36)     NOT NULL,
    purchase_order_id CHAR(36)     NOT NULL,
    delivery_code     VARCHAR(40)  NULL,
    received_by       CHAR(36)     NOT NULL,
    notes             VARCHAR(500) NULL,
    received_at       DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_supplier_delivery PRIMARY KEY (id),
    CONSTRAINT fk_delivery_order    FOREIGN KEY (purchase_order_id) REFERENCES `PURCHASE_ORDER` (id),
    CONSTRAINT idx_delivery_order   INDEX (purchase_order_id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- sales
CREATE TABLE `TRANSACTION` (
    id             CHAR(36)      NOT NULL,
    receipt_number VARCHAR(40)   NOT NULL,
    store_id       CHAR(36)      NOT NULL,
    cashier_id     CHAR(36)      NOT NULL,
    customer_id    CHAR(36)      NULL,
    subtotal       DECIMAL(14,2) NOT NULL DEFAULT 0.00,
    discount_total DECIMAL(14,2) NOT NULL DEFAULT 0.00,
    tax_total      DECIMAL(14,2) NOT NULL DEFAULT 0.00,
    grand_total    DECIMAL(14,2) NOT NULL DEFAULT 0.00,
    status         ENUM ('COMPLETED','VOIDED','REFUNDED') NOT NULL DEFAULT 'COMPLETED',
    voided_by      CHAR(36)      NULL,
    void_reason    VARCHAR(255)  NULL,
    created_at     DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_transaction PRIMARY KEY (id),
    CONSTRAINT uq_transaction_receipt UNIQUE (receipt_number),
    CONSTRAINT fk_transaction_store    FOREIGN KEY (store_id)    REFERENCES `STORE` (id),
    CONSTRAINT fk_transaction_cashier  FOREIGN KEY (cashier_id)  REFERENCES `USER` (id),
    CONSTRAINT fk_transaction_customer FOREIGN KEY (customer_id) REFERENCES `CUSTOMER` (id),
    CONSTRAINT idx_transaction_created INDEX (created_at),
    CONSTRAINT idx_transaction_store_date INDEX (store_id, created_at)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

CREATE TABLE `TRANSACTION_ITEM` (
    id            CHAR(36)      NOT NULL,
    transaction_id CHAR(36)     NOT NULL,
    product_id    CHAR(36)      NOT NULL,
    quantity      INT           NOT NULL,
    unit_price    DECIMAL(12,2) NOT NULL,
    cost_price    DECIMAL(12,2) NOT NULL DEFAULT 0.00,
    line_discount DECIMAL(12,2) NOT NULL DEFAULT 0.00,
    tax_rate      DECIMAL(5,2)  NOT NULL DEFAULT 0.00,
    line_total    DECIMAL(14,2) NOT NULL,
    CONSTRAINT pk_transaction_item PRIMARY KEY (id),
    CONSTRAINT fk_ti_transaction FOREIGN KEY (transaction_id) REFERENCES `TRANSACTION` (id) ON DELETE CASCADE,
    CONSTRAINT fk_ti_product     FOREIGN KEY (product_id)     REFERENCES `PRODUCT` (id),
    CONSTRAINT idx_ti_product INDEX (product_id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- One-to-many: split payments (cash + card + digital wallet on one ticket).
CREATE TABLE `PAYMENT` (
    id             CHAR(36)      NOT NULL,
    transaction_id CHAR(36)      NOT NULL,
    method         ENUM ('CASH','CARD','DIGITAL_WALLET') NOT NULL,
    amount         DECIMAL(12,2) NOT NULL,
    reference      VARCHAR(80)   NULL,
    created_at     DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_payment PRIMARY KEY (id),
    CONSTRAINT fk_payment_transaction FOREIGN KEY (transaction_id) REFERENCES `TRANSACTION` (id) ON DELETE CASCADE
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- stock ops audit
-- Shrinkage, damage, returns, corrections and store-to-store transfers.
-- Sale deductions are audited via TRANSACTION_ITEM; this table is the ops ledger.
CREATE TABLE `STOCK_ADJUSTMENT` (
    id              CHAR(36)     NOT NULL,
    product_id      CHAR(36)     NOT NULL,
    store_id        CHAR(36)     NOT NULL,
    batch_id        CHAR(36)     NULL,
    adjustment_type ENUM ('SALE','PURCHASE','SHRINKAGE','DAMAGE','RETURN','TRANSFER_IN','TRANSFER_OUT','CORRECTION') NOT NULL,
    quantity_delta  INT          NOT NULL COMMENT 'negative = stock out, positive = stock in',
    unit_cost       DECIMAL(12,2) NOT NULL DEFAULT 0.00,
    reason          VARCHAR(255) NULL,
    created_by      CHAR(36)     NULL,
    created_at      DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_stock_adjustment PRIMARY KEY (id),
    CONSTRAINT fk_adjustment_product FOREIGN KEY (product_id) REFERENCES `PRODUCT` (id),
    CONSTRAINT fk_adjustment_store   FOREIGN KEY (store_id)   REFERENCES `STORE` (id),
    CONSTRAINT fk_adjustment_batch   FOREIGN KEY (batch_id)   REFERENCES `STOCK_BATCH` (id),
    CONSTRAINT fk_adjustment_user    FOREIGN KEY (created_by) REFERENCES `USER` (id),
    CONSTRAINT idx_adjustment_product_date INDEX (product_id, created_at)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- ---------------------------------------------------------------- history
CREATE TABLE `PRODUCT_PRICE_HISTORY` (
    id               CHAR(36)      NOT NULL,
    product_id       CHAR(36)      NOT NULL,
    old_cost_price   DECIMAL(12,2) NOT NULL,
    old_selling_price DECIMAL(12,2) NOT NULL,
    new_cost_price   DECIMAL(12,2) NOT NULL,
    new_selling_price DECIMAL(12,2) NOT NULL,
    changed_by       CHAR(36)      NULL,
    changed_at       DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_product_price_history PRIMARY KEY (id),
    CONSTRAINT fk_price_history_product FOREIGN KEY (product_id) REFERENCES `PRODUCT` (id),
    CONSTRAINT fk_price_history_user    FOREIGN KEY (changed_by) REFERENCES `USER` (id),
    CONSTRAINT idx_price_history_product INDEX (product_id, changed_at)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

-- -----------------------------------------------------------------
CREATE TABLE `CUSTOMER_LOYALTY_LOG` (
    id             CHAR(36)     NOT NULL,
    customer_id    CHAR(36)     NOT NULL,
    points_delta   INT          NOT NULL,
    reason         VARCHAR(255) NOT NULL,
    transaction_id CHAR(36)     NULL,
    created_by     CHAR(36)     NULL,
    created_at     DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_customer_loyalty_log PRIMARY KEY (id),
    CONSTRAINT fk_loyalty_customer FOREIGN KEY (customer_id)    REFERENCES `CUSTOMER` (id),
    CONSTRAINT fk_loyalty_tx       FOREIGN KEY (transaction_id) REFERENCES `TRANSACTION` (id),
    CONSTRAINT fk_loyalty_user     FOREIGN KEY (created_by)     REFERENCES `USER` (id),
    CONSTRAINT idx_loyalty_customer INDEX (customer_id, created_at)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;

CREATE TABLE `NOTIFICATION` (
    id           CHAR(36)     NOT NULL,
    type_name    ENUM ('LOW_STOCK','PO_STATUS','DELIVERY','SYSTEM') NOT NULL,
    title        VARCHAR(120) NOT NULL,
    message      VARCHAR(500) NOT NULL,
    audience     ENUM ('ALL','ADMIN','MANAGER','INVENTORY_MANAGER','CASHIER','SUPPLIER') NOT NULL DEFAULT 'ALL',
    entity_type  VARCHAR(40)  NULL,
    entity_id    CHAR(36)     NULL,
    is_read      BOOLEAN      NOT NULL DEFAULT FALSE,
    created_at   DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT pk_notification PRIMARY KEY (id),
    CONSTRAINT idx_notification_audience INDEX (audience, is_read, created_at)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4;
