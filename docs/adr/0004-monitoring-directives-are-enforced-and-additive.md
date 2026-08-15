# Monitoring directives are enforced, and additive only

An operational authority holder can require named roles to monitor named loops — before a critical pass, say — and the operators subscribed by it cannot drop those loops while the directive is in force. Acknowledgement is a read receipt confirming the operator has seen that their console changed, not consent that could be withheld.

An advisory version was considered and rejected: if the point is to be sure everyone is listening to the right thing during a critical operation, a request that can be declined does not deliver it, and the admin discovers the decline at the worst possible moment.

Enforcement is only safe because directives are **additive**. Adding a loop to an operator's console never removes information from them, so there is no case where the directive itself causes something to be missed. An exclusive form — where the named set replaces the operator's subscriptions — was rejected for exactly that reason: it could silently drop the one loop where the operator's actual problem was unfolding.

## Consequences

- A directive targets roles, not users, so it binds whoever occupies them, including operators who sign in after it was issued. Anything else fails quietly for late arrivals.
- Directives do not auto-expire; one timing out mid-pass would be a hazard. They stay in force until a holder of operational authority clears them.
- Operators get more in their ears at the busiest moment and cannot shed it. Directed loops must therefore be visibly marked as directed, so nobody mistakes them for their own choice.
