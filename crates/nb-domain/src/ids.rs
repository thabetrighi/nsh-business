//! Typed identifiers of the business entities.

use nb_core::typed_id;

typed_id!(
    /// A branch (store, depot) of an organization.
    BranchId
);
typed_id!(
    /// A warehouse or stock location.
    WarehouseId
);
typed_id!(
    /// A user account.
    UserId
);
typed_id!(
    /// A product or service.
    ProductId
);
typed_id!(
    /// A product category.
    CategoryId
);
typed_id!(
    /// A unit of measure.
    UnitId
);
typed_id!(
    /// A customer.
    CustomerId
);
typed_id!(
    /// A supplier.
    SupplierId
);
typed_id!(
    /// A stock movement (append-only).
    MovementId
);
typed_id!(
    /// A journal entry.
    EntryId
);
typed_id!(
    /// A sales, purchase or stock document.
    DocumentId
);
typed_id!(
    /// A cash register (point of sale till).
    RegisterId
);
typed_id!(
    /// A role defined by an organization.
    RoleId
);
typed_id!(
    /// A supervisor's approval of a sensitive action.
    ApprovalId
);
