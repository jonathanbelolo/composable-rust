<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { Store } from '@composable-svelte/core';
  import type { AppState, Event, EventSummary } from './types';
  import { initialState } from './types';
  import type { AppAction } from './reducer';
  import { urls } from './routing';

  // Import composable-svelte UI components
  import {
    Button,
    Card,
    CardHeader,
    CardTitle,
    CardDescription,
    CardContent,
    Input,
    Badge,
    Spinner,
    Skeleton,
    Progress,
    DropdownMenu
  } from '@composable-svelte/core/components/ui';
  import type { MenuItem } from '@composable-svelte/core/components/ui/dropdown-menu';

  // Navigation components can be imported from '@composable-svelte/core/navigation-components' when needed
  // Available: Modal, Sheet, Alert, Drawer, Popover, Sidebar, Tabs, NavigationStack

  // Props
  interface Props {
    store: Store<AppState, AppAction>;
  }

  let { store }: Props = $props();

  // ============================================================================
  // Reactive State Bridge
  // ============================================================================
  // The store uses a subscription-based pattern, not $state runes internally.
  // Per composable-svelte examples (product-gallery/App.svelte):
  // "Use onMount + subscribe instead of $derived to avoid infinite loops"
  // We bridge the store's updates to Svelte 5's reactive system via $state.
  let state = $state<AppState>(store.state);

  onMount(() => {
    // Subscribe to store updates
    const unsubscribe = store.subscribe((s) => (state = s));

    // Handle magic link verification on mount (after hydration)
    // This runs once when the component mounts on the client
    const currentState = store.state;
    if (currentState.destination.type === 'verify' && !currentState.auth.isAuthenticated) {
      const params = new URLSearchParams(window.location.search);
      const token = params.get('token');
      if (token) {
        console.log('[Auth] Verifying magic link token...');
        verificationAttempted = true;
        store.dispatch({ type: 'auth/verifyToken', token });
      }
    }

    return unsubscribe;
  });

  // Derived values for convenience (now reactive via state)
  const destination = $derived(state.destination);
  const auth = $derived(state.auth);
  const meta = $derived(state.meta);
  const events = $derived(state.events);
  const eventsLoading = $derived(state.eventsLoading);
  const eventsError = $derived(state.eventsError);
  const selectedEvent = $derived(state.selectedEvent);
  const availability = $derived(state.availability);
  const reservations = $derived(state.reservations);
  const currentReservation = $derived(state.currentReservation);
  const reservationsLoading = $derived(state.reservationsLoading);
  const reservationsError = $derived(state.reservationsError);
  const payments = $derived(state.payments);
  const paymentsLoading = $derived(state.paymentsLoading);
  const paymentsError = $derived(state.paymentsError);
  const organizer = $derived(state.organizer);
  const toasts = $derived(state.ui.toasts);

  // Track if we've already attempted verification to prevent loops
  let verificationAttempted = $state(false);

  // Track toasts we've already set auto-dismiss timers for
  let toastTimersSet = $state(new Set<string>());

  // User dropdown menu items (dynamically updated based on auth)
  // Note: icon property renders as text content, so use Unicode/emoji symbols
  const userMenuItems = $derived<MenuItem[]>([
    { id: 'user-email', label: auth.user?.email ?? 'User', disabled: true },
    { id: 'sep1', label: '', isSeparator: true },
    { id: 'dashboard', label: 'Dashboard', icon: '📊' },
    { id: 'organize', label: 'Organize Events', icon: '📅' },
    { id: 'sep2', label: '', isSeparator: true },
    { id: 'logout', label: 'Sign out', icon: '🚪' }
  ]);

  // Handle user menu item selection
  function handleUserMenuSelect(item: MenuItem) {
    switch (item.id) {
      case 'dashboard':
        navigate({ type: 'dashboard', state: {} });
        break;
      case 'organize':
        navigate({ type: 'organizerEvents', state: {} });
        break;
      case 'logout':
        store.dispatch({ type: 'auth/logout' });
        break;
    }
  }

  // Auto-load data when navigating to certain pages
  // Note: Dashboard/myReservations and organizerEvents data loading is handled
  // by the reducer when processing 'navigate' actions (proper composable architecture)
  $effect(() => {
    // Load events list when navigating to events page (including direct URL access)
    if (destination.type === 'events' && events.length === 0 && !eventsLoading && !eventsError) {
      store.dispatch({ type: 'events/loadList' });
    }
    // Load event detail when navigating to event detail page
    if (destination.type === 'eventDetail' && destination.state.eventId && !selectedEvent && !eventsLoading && !eventsError) {
      store.dispatch({ type: 'events/loadDetail', eventId: destination.state.eventId });
    }
    // Note: Magic link verification is handled in onMount to ensure it runs after hydration
    // Auto-load event for editing
    if (destination.type === 'editEvent' &&
        destination.state.eventId &&
        !organizer.formLoading) {
      store.dispatch({ type: 'organizer/loadEventForEdit', eventId: destination.state.eventId });
    }
    // Reset form when entering create event page
    if (destination.type === 'createEvent' && organizer.eventForm.title !== '') {
      store.dispatch({ type: 'organizer/resetForm' });
    }
  });

  // Auto-dismiss toasts after 5 seconds
  $effect(() => {
    if (typeof window === 'undefined') return; // Only run in browser
    if (toasts.length === 0) return;

    for (const toast of toasts) {
      if (toastTimersSet.has(toast.id)) continue;

      // Mark this toast as having a timer set
      toastTimersSet = new Set([...toastTimersSet, toast.id]);

      const toastId = toast.id;
      setTimeout(() => {
        store.dispatch({ type: 'ui/dismissToast', id: toastId });
        // Clean up the tracked ID
        toastTimersSet = new Set([...toastTimersSet].filter((id) => id !== toastId));
      }, 5000);
    }
  });

  // Navigation helper - dispatches navigate action
  function navigate(dest: AppState['destination']) {
    store.dispatch({ type: 'navigate', destination: dest });
  }

  // Format helpers
  function formatDate(dateString: string): string {
    return new Date(dateString).toLocaleDateString('en-US', {
      weekday: 'short',
      month: 'short',
      day: 'numeric'
    });
  }

  function formatTime(dateString: string): string {
    return new Date(dateString).toLocaleTimeString('en-US', {
      hour: 'numeric',
      minute: '2-digit'
    });
  }

  function formatPrice(cents: number): string {
    return `$${(cents / 100).toFixed(0)}`;
  }

  function getLowestPrice(event: Event): number {
    const prices = event.pricing_tiers.map(t => t.price_cents);
    return Math.min(...prices);
  }

  function getEventName(eventId: string): string {
    const event = events.find(e => e.id === eventId);
    return event?.title ?? 'Unknown Event';
  }
</script>

<svelte:head>
  <title>{meta.title}</title>
  <meta name="description" content={meta.description} />
  {#if meta.canonical}
    <link rel="canonical" href={meta.canonical} />
  {/if}
  {#if meta.ogImage}
    <meta property="og:image" content={meta.ogImage} />
  {/if}
</svelte:head>

<div class="min-h-screen bg-gray-50">
  <!-- Header -->
  <header class="bg-white shadow-sm">
    <nav class="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
      <div class="flex h-16 justify-between items-center">
        <div class="flex items-center space-x-8">
          <a
            href={urls.home()}
            class="text-xl font-bold text-primary-600"
            onclick={(e: MouseEvent) => {
              e.preventDefault();
              navigate({ type: 'home', state: {} });
            }}
          >
            Ticketing
          </a>
          <a
            href={urls.events()}
            class="text-gray-600 hover:text-gray-900"
            onclick={(e: MouseEvent) => {
              e.preventDefault();
              navigate({ type: 'events', state: {} });
              store.dispatch({ type: 'events/loadList' });
            }}
          >
            Events
          </a>
        </div>

        <div class="flex items-center space-x-4">
          {#if auth.isAuthenticated}
            <!-- User dropdown menu -->
            <DropdownMenu
              items={userMenuItems}
              onSelect={handleUserMenuSelect}
              align="end"
            >
              <button
                type="button"
                class="flex items-center gap-2 px-3 py-2 text-gray-700 hover:text-gray-900 hover:bg-gray-100 rounded-lg transition-colors"
              >
                <!-- User icon -->
                <svg class="w-6 h-6 text-gray-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M17.982 18.725A7.488 7.488 0 0012 15.75a7.488 7.488 0 00-5.982 2.975m11.963 0a9 9 0 10-11.963 0m11.963 0A8.966 8.966 0 0112 21a8.966 8.966 0 01-5.982-2.275M15 9.75a3 3 0 11-6 0 3 3 0 016 0z" />
                </svg>
                <!-- Chevron down -->
                <svg class="w-4 h-4 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M19 9l-7 7-7-7" />
                </svg>
              </button>
            </DropdownMenu>
          {:else}
            <Button
              onclick={() => navigate({ type: 'login', state: {} })}
            >
              Sign In
            </Button>
          {/if}
        </div>
      </div>
    </nav>
  </header>

  <!-- Main Content -->
  <main class="mx-auto max-w-7xl px-4 py-8 sm:px-6 lg:px-8">
    {#if destination.type === 'home'}
      <!-- HOME PAGE -->
      <div class="text-center py-16">
        <h1 class="text-4xl font-bold text-gray-900 mb-4">
          Find Your Next Event
        </h1>
        <p class="text-xl text-gray-600 mb-8">
          Discover and book tickets for concerts, sports, theater, and more.
        </p>
        <Button
          size="lg"
          onclick={() => {
            navigate({ type: 'events', state: {} });
            store.dispatch({ type: 'events/loadList' });
          }}
        >
          Browse Events
        </Button>
      </div>

    {:else if destination.type === 'events'}
      <!-- EVENTS LIST PAGE -->
      <div>
        <h1 class="text-3xl font-bold text-gray-900 mb-6">Events</h1>

        {#if eventsLoading}
          <div class="flex justify-center py-12">
            <Spinner size="lg" />
          </div>
        {:else if eventsError}
          <Card>
            <CardContent>
              <p class="text-red-600">{eventsError}</p>
              <Button
                variant="outline"
                class="mt-4"
                onclick={() => store.dispatch({ type: 'events/loadList' })}
              >
                Try Again
              </Button>
            </CardContent>
          </Card>
        {:else if events.length === 0}
          <Card>
            <CardContent class="text-center py-12">
              <p class="text-gray-600">No events found.</p>
            </CardContent>
          </Card>
        {:else}
          <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
            {#each events as event (event.id)}
              <Card class="overflow-hidden hover:shadow-lg transition-shadow">
                <CardHeader>
                  <CardTitle>{event.title}</CardTitle>
                  <CardDescription>
                    {event.venue.name} &middot; {formatDate(event.start_time)}
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div class="flex justify-between items-center">
                    <Badge variant={event.status === 'published' ? 'success' : 'secondary'}>
                      {event.status}
                    </Badge>
                    <Button
                      size="sm"
                      onclick={() => {
                        navigate({ type: 'eventDetail', state: { eventId: event.id } });
                        store.dispatch({ type: 'events/loadDetail', eventId: event.id });
                      }}
                    >
                      Get Tickets
                    </Button>
                  </div>
                </CardContent>
              </Card>
            {/each}
          </div>
        {/if}
      </div>

    {:else if destination.type === 'eventDetail'}
      <!-- EVENT DETAIL PAGE -->
      <div>
        {#if eventsLoading}
          <div class="flex justify-center py-12">
            <Spinner size="lg" />
          </div>
        {:else if eventsError}
          <Card class="max-w-md mx-auto mt-12">
            <CardHeader>
              <CardTitle class="text-red-600">Unable to Load Event</CardTitle>
            </CardHeader>
            <CardContent>
              <p class="text-gray-600 mb-4">{eventsError}</p>
              <Button variant="outline" onclick={() => navigate({ type: 'events', state: {} })}>
                Back to Events
              </Button>
            </CardContent>
          </Card>
        {:else if selectedEvent}
          {@const event = selectedEvent}
          <div class="grid grid-cols-1 lg:grid-cols-3 gap-8">
            <!-- Main Info -->
            <div class="lg:col-span-2">
              <Card>
                <CardHeader>
                  <div class="flex items-start justify-between">
                    <div>
                      <CardTitle class="text-3xl">{event.title}</CardTitle>
                      <CardDescription class="mt-2">
                        {event.venue.name} &middot; {event.venue.address}
                      </CardDescription>
                    </div>
                    <Badge variant={event.status === 'published' ? 'success' : 'secondary'}>
                      {event.status === 'published' ? 'On Sale' : event.status}
                    </Badge>
                  </div>
                </CardHeader>
                <CardContent>
                  <div class="space-y-4">
                    <div>
                      <h3 class="font-semibold text-gray-900">Date & Time</h3>
                      <p class="text-gray-600">
                        {formatDate(event.start_time)} at {formatTime(event.start_time)}
                      </p>
                    </div>
                    {#if event.description}
                      <div>
                        <h3 class="font-semibold text-gray-900">About</h3>
                        <p class="text-gray-600">{event.description}</p>
                      </div>
                    {/if}
                  </div>
                </CardContent>
              </Card>
            </div>

            <!-- Pricing Sidebar -->
            <div>
              <Card>
                <CardHeader>
                  <CardTitle>Tickets</CardTitle>
                </CardHeader>
                <CardContent>
                  <div class="space-y-3">
                    {#each event.pricing_tiers as tier}
                      <div class="flex justify-between items-center p-3 bg-gray-50 rounded-lg">
                        <div>
                          <div class="font-medium">{tier.name}</div>
                          {#if tier.quantity_available !== null}
                            <div class="text-xs text-gray-500">{tier.quantity_available} left</div>
                          {/if}
                        </div>
                        <div class="text-lg font-bold">{formatPrice(tier.price_cents)}</div>
                      </div>
                    {/each}
                  </div>

                  <Button
                    class="w-full mt-6"
                    size="lg"
                    onclick={() => {
                      if (auth.isAuthenticated) {
                        navigate({ type: 'reserve', state: { eventId: event.id } });
                        store.dispatch({ type: 'reservations/startFlow', eventId: event.id });
                      } else {
                        navigate({ type: 'login', state: {} });
                      }
                    }}
                  >
                    {auth.isAuthenticated ? 'Get Tickets' : 'Sign In to Book'}
                  </Button>
                </CardContent>
              </Card>

              {#if availability && availability.length > 0}
                <Card class="mt-4">
                  <CardHeader>
                    <CardTitle>Availability</CardTitle>
                  </CardHeader>
                  <CardContent>
                    {#each availability as section}
                      <div class="mb-3">
                        <div class="flex justify-between text-sm mb-1">
                          <span>{section.section}</span>
                          <span>{section.available_seats}/{section.total_capacity}</span>
                        </div>
                        <Progress value={(section.available_seats / section.total_capacity) * 100} />
                      </div>
                    {/each}
                  </CardContent>
                </Card>
              {/if}
            </div>
          </div>
        {:else}
          <Card>
            <CardContent class="text-center py-12">
              <p class="text-gray-600">Event not found.</p>
              <Button
                variant="outline"
                class="mt-4"
                onclick={() => {
                  navigate({ type: 'events', state: {} });
                  store.dispatch({ type: 'events/loadList' });
                }}
              >
                Browse Events
              </Button>
            </CardContent>
          </Card>
        {/if}
      </div>

    {:else if destination.type === 'login'}
      <!-- LOGIN PAGE -->
      <div class="max-w-md mx-auto">
        <Card>
          <CardHeader class="text-center">
            <CardTitle>Sign In</CardTitle>
            <CardDescription>Enter your email to receive a magic link</CardDescription>
          </CardHeader>
          <CardContent>
            {#if auth.magicLinkSent}
              <div class="text-center py-4">
                <div class="text-green-600 mb-2">
                  {auth.testMagicLink ? 'Magic link generated!' : 'Check your email!'}
                </div>
                <p class="text-gray-600 text-sm">
                  {auth.testMagicLink
                    ? 'Click the link below to sign in (testing mode):'
                    : 'We\'ve sent you a magic link. Click it to sign in.'}
                </p>
                {#if auth.testMagicLink}
                  <a
                    href={auth.testMagicLink}
                    class="mt-4 inline-block px-4 py-2 bg-primary-600 text-white rounded-md hover:bg-primary-700 transition-colors"
                  >
                    Click to Sign In
                  </a>
                  <p class="mt-2 text-xs text-gray-400 break-all">
                    {auth.testMagicLink}
                  </p>
                {/if}
              </div>
            {:else}
              <form
                onsubmit={(e: Event) => {
                  e.preventDefault();
                  const formData = new FormData(e.currentTarget as HTMLFormElement);
                  const email = formData.get('email') as string;
                  store.dispatch({ type: 'auth/requestMagicLink', email });
                }}
              >
                <div class="space-y-4">
                  <Input
                    type="email"
                    name="email"
                    placeholder="you@example.com"
                    required
                    disabled={auth.isLoading}
                  />

                  {#if auth.error}
                    <p class="text-red-600 text-sm">{auth.error}</p>
                  {/if}

                  <Button type="submit" class="w-full" disabled={auth.isLoading}>
                    {#if auth.isLoading}
                      <Spinner size="sm" /> Sending...
                    {:else}
                      Send Magic Link
                    {/if}
                  </Button>
                </div>
              </form>
            {/if}
          </CardContent>
        </Card>
      </div>

    {:else if destination.type === 'dashboard'}
      <!-- DASHBOARD PAGE -->
      <div>
        <h1 class="text-3xl font-bold text-gray-900 mb-6">Dashboard</h1>

        <!-- Simple tab navigation without the Tabs component -->
        <div class="border-b border-gray-200 mb-6">
          <nav class="-mb-px flex space-x-8">
            <button
              class="py-2 px-1 border-b-2 font-medium text-sm {destination.state?.tab === 'payments' ? 'border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300' : 'border-primary-500 text-primary-600'}"
              onclick={() => {
                navigate({ type: 'dashboard', state: { tab: 'reservations' } });
                store.dispatch({ type: 'reservations/loadMine' });
              }}
            >
              My Reservations
            </button>
            <button
              class="py-2 px-1 border-b-2 font-medium text-sm {destination.state?.tab === 'payments' ? 'border-primary-500 text-primary-600' : 'border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300'}"
              onclick={() => {
                navigate({ type: 'dashboard', state: { tab: 'payments' } });
                store.dispatch({ type: 'payments/loadHistory' });
              }}
            >
              Payment History
            </button>
          </nav>
        </div>

        {#if destination.state?.tab === 'payments'}
          {#if paymentsLoading && payments.length === 0}
            <div class="flex justify-center py-12">
              <Spinner size="lg" />
            </div>
          {:else if !paymentsLoading && payments.length === 0}
            <Card>
              <CardContent class="text-center py-12">
                <p class="text-gray-600 mb-2">No payment history yet.</p>
                <p class="text-sm text-gray-500">Payments will appear here after you complete a purchase.</p>
              </CardContent>
            </Card>
          {:else}
            <div class="space-y-4">
              {#each payments as payment (payment.id)}
                <Card>
                  <CardContent class="py-4">
                    <div class="flex justify-between items-start">
                      <div>
                        <div class="font-medium">{formatPrice(payment.amount_cents)}</div>
                        <div class="text-sm text-gray-500">
                          {payment.payment_method}
                        </div>
                        {#if payment.transaction_id}
                          <div class="text-xs text-gray-400">
                            Transaction: {payment.transaction_id}
                          </div>
                        {/if}
                      </div>
                      <Badge variant={
                        payment.status === 'succeeded' ? 'success' :
                        payment.status === 'processing' ? 'warning' :
                        payment.status === 'failed' ? 'destructive' :
                        'secondary'
                      }>
                        {payment.status}
                      </Badge>
                    </div>
                    {#if payment.failure_reason}
                      <p class="mt-2 text-sm text-red-600">{payment.failure_reason}</p>
                    {/if}
                  </CardContent>
                </Card>
              {/each}
            </div>
          {/if}
        {:else}
          {#if reservationsLoading && reservations.length === 0}
            <div class="flex justify-center py-12">
              <Spinner size="lg" />
            </div>
          {:else if !reservationsLoading && reservations.length === 0}
            <Card>
              <CardContent class="py-8 text-center text-gray-600">
                <p>You don't have any reservations yet.</p>
                <Button
                  variant="outline"
                  class="mt-4"
                  onclick={() => {
                    navigate({ type: 'events', state: {} });
                    store.dispatch({ type: 'events/loadList' });
                  }}
                >
                  Browse Events
                </Button>
              </CardContent>
            </Card>
          {:else}
            <div class="space-y-4">
              {#each reservations as reservation (reservation.id)}
                <Card>
                  <CardContent class="py-4">
                    <div class="flex justify-between items-start">
                      <div>
                        <div class="font-medium text-lg">{getEventName(reservation.event_id)}</div>
                        <div class="text-sm text-gray-500">
                          {reservation.quantity} ticket(s) in {reservation.section} - {formatPrice(reservation.total_amount_cents)}
                        </div>
                        <div class="text-sm text-gray-400">
                          {new Date(reservation.created_at).toLocaleDateString()} &middot; #{reservation.id.slice(0, 8)}
                        </div>
                      </div>
                      <Badge variant={
                        reservation.status === 'Confirmed' || reservation.status === 'confirmed' || reservation.status === 'completed' ? 'success' :
                        reservation.status === 'Pending' || reservation.status === 'pending' || reservation.status === 'PaymentPending' ? 'warning' :
                        'secondary'
                      }>
                        {reservation.status}
                      </Badge>
                    </div>
                    {#if reservation.status === 'Pending' || reservation.status === 'pending' || reservation.status === 'PaymentPending'}
                      <div class="mt-3 flex gap-2">
                        <Button
                          size="sm"
                          onclick={() => {
                            navigate({ type: 'reserve', state: { eventId: reservation.event_id } });
                          }}
                        >
                          Complete Payment
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          onclick={() => store.dispatch({ type: 'reservations/cancel', reservationId: reservation.id })}
                        >
                          Cancel
                        </Button>
                      </div>
                    {/if}
                  </CardContent>
                </Card>
              {/each}
            </div>
          {/if}
        {/if}
      </div>

    {:else if destination.type === 'reserve'}
      <!-- RESERVATION FLOW PAGE -->
      <div>
        {#if !currentReservation}
          <Card>
            <CardContent class="text-center py-12">
              <p class="text-gray-600">No reservation in progress.</p>
              <Button
                variant="outline"
                class="mt-4"
                onclick={() => {
                  navigate({ type: 'events', state: {} });
                  store.dispatch({ type: 'events/loadList' });
                }}
              >
                Browse Events
              </Button>
            </CardContent>
          </Card>
        {:else if currentReservation.step === 'select-tickets'}
          <!-- Step 1: Select Tickets -->
          <div class="max-w-2xl mx-auto">
            <h1 class="text-3xl font-bold text-gray-900 mb-6">Select Tickets</h1>

            {#if selectedEvent}
              <Card class="mb-6">
                <CardHeader>
                  <CardTitle>{selectedEvent.title}</CardTitle>
                  <CardDescription>
                    {selectedEvent.venue.name} - {formatDate(selectedEvent.start_time)}
                  </CardDescription>
                </CardHeader>
              </Card>

              <Card>
                <CardHeader>
                  <CardTitle>Available Tiers</CardTitle>
                </CardHeader>
                <CardContent>
                  <div class="space-y-4">
                    {#each selectedEvent.pricing_tiers as tier}
                      <div class="flex justify-between items-center p-4 bg-gray-50 rounded-lg">
                        <div>
                          <div class="font-medium">{tier.name}</div>
                          <div class="text-sm text-gray-500">
                            {formatPrice(tier.price_cents)}
                            {#if tier.quantity_available !== null}
                              - {tier.quantity_available} available
                            {/if}
                          </div>
                        </div>
                        <Button
                          size="sm"
                          onclick={() => {
                            store.dispatch({
                              type: 'reservations/selectSeat',
                              seat: {
                                seatId: crypto.randomUUID(),
                                section: selectedEvent.venue.sections[0]?.name ?? 'General',
                                tierType: tier.tier_type,
                                priceCents: tier.price_cents
                              }
                            });
                          }}
                        >
                          Add
                        </Button>
                      </div>
                    {/each}
                  </div>

                  {#if currentReservation.selectedSeats.length > 0}
                    <div class="mt-6 pt-6 border-t">
                      <h4 class="font-semibold mb-3">Selected Tickets</h4>
                      <div class="space-y-2">
                        {#each currentReservation.selectedSeats as seat, i}
                          <div class="flex justify-between items-center text-sm">
                            <span>{seat.tierType} - {seat.section}</span>
                            <div class="flex items-center gap-2">
                              <span>{formatPrice(seat.priceCents)}</span>
                              <button
                                class="text-red-500 hover:text-red-700"
                                onclick={() => store.dispatch({ type: 'reservations/deselectSeat', seatId: seat.seatId })}
                              >
                                Remove
                              </button>
                            </div>
                          </div>
                        {/each}
                      </div>
                      <div class="mt-4 flex justify-between items-center font-semibold">
                        <span>Total</span>
                        <span>{formatPrice(currentReservation.selectedSeats.reduce((sum, s) => sum + s.priceCents, 0))}</span>
                      </div>
                    </div>
                  {/if}

                  <div class="mt-6 flex gap-3">
                    <Button
                      variant="outline"
                      onclick={() => {
                        navigate({ type: 'eventDetail', state: { eventId: currentReservation.eventId } });
                      }}
                    >
                      Back
                    </Button>
                    <Button
                      class="flex-1"
                      disabled={currentReservation.selectedSeats.length === 0 || reservationsLoading}
                      onclick={() => store.dispatch({ type: 'reservations/create' })}
                    >
                      {#if reservationsLoading}
                        <Spinner size="sm" /> Reserving...
                      {:else}
                        Continue to Payment
                      {/if}
                    </Button>
                  </div>

                  {#if reservationsError}
                    <p class="mt-4 text-red-600 text-sm">{reservationsError}</p>
                  {/if}
                </CardContent>
              </Card>
            {:else}
              <div class="flex justify-center py-12">
                <Spinner size="lg" />
              </div>
            {/if}
          </div>

        {:else if currentReservation.step === 'payment'}
          <!-- Step 2: Payment -->
          <div class="max-w-md mx-auto">
            <h1 class="text-3xl font-bold text-gray-900 mb-6">Payment</h1>

            <Card>
              <CardHeader>
                <CardTitle>Complete Your Order</CardTitle>
                {#if currentReservation.expiresAt}
                  <CardDescription>
                    Reservation expires at {formatTime(currentReservation.expiresAt)}
                  </CardDescription>
                {/if}
              </CardHeader>
              <CardContent>
                <div class="space-y-4">
                  <div class="p-4 bg-gray-50 rounded-lg">
                    <div class="flex justify-between mb-2">
                      <span>Tickets ({currentReservation.selectedSeats.length})</span>
                      <span>{formatPrice(currentReservation.selectedSeats.reduce((sum, s) => sum + s.priceCents, 0))}</span>
                    </div>
                    <div class="flex justify-between font-semibold">
                      <span>Total</span>
                      <span>{formatPrice(currentReservation.selectedSeats.reduce((sum, s) => sum + s.priceCents, 0))}</span>
                    </div>
                  </div>

                  <div class="space-y-2">
                    <Button
                      class="w-full"
                      disabled={paymentsLoading}
                      onclick={() => {
                        if (currentReservation.reservationId) {
                          store.dispatch({
                            type: 'payments/process',
                            reservationId: currentReservation.reservationId,
                            paymentMethod: 'credit_card'
                          });
                        }
                      }}
                    >
                      {#if paymentsLoading}
                        <Spinner size="sm" /> Processing...
                      {:else}
                        Pay with Credit Card
                      {/if}
                    </Button>
                  </div>

                  {#if paymentsError}
                    <p class="text-red-600 text-sm">{paymentsError}</p>
                  {/if}
                </div>
              </CardContent>
            </Card>
          </div>

        {:else if currentReservation.step === 'complete'}
          <!-- Step 3: Complete -->
          <div class="max-w-md mx-auto text-center">
            <Card>
              <CardContent class="py-12">
                <div class="text-green-600 text-5xl mb-4">&#10003;</div>
                <h1 class="text-2xl font-bold text-gray-900 mb-2">Payment Successful!</h1>
                <p class="text-gray-600 mb-6">Your tickets have been confirmed.</p>
                <div class="space-y-3">
                  <Button onclick={() => navigate({ type: 'myReservations', state: {} })}>
                    View My Reservations
                  </Button>
                  <Button
                    variant="outline"
                    onclick={() => {
                      navigate({ type: 'events', state: {} });
                      store.dispatch({ type: 'events/loadList' });
                    }}
                  >
                    Browse More Events
                  </Button>
                </div>
              </CardContent>
            </Card>
          </div>
        {/if}
      </div>

    {:else if destination.type === 'verify'}
      <!-- VERIFY MAGIC LINK PAGE -->
      <div class="max-w-md mx-auto">
        <Card>
          <CardHeader class="text-center">
            {#if auth.isAuthenticated}
              <CardTitle>Welcome!</CardTitle>
              <CardDescription>You've been logged in successfully</CardDescription>
            {:else if auth.error}
              <CardTitle>Verification Failed</CardTitle>
              <CardDescription>We couldn't verify your login</CardDescription>
            {:else if auth.isLoading}
              <CardTitle>Verifying...</CardTitle>
              <CardDescription>Please wait while we verify your login</CardDescription>
            {:else}
              <CardTitle>Magic Link</CardTitle>
              <CardDescription>Waiting for verification token</CardDescription>
            {/if}
          </CardHeader>
          <CardContent class="text-center">
            {#if auth.isLoading}
              <Spinner size="lg" />
            {:else if auth.error}
              <div>
                <p class="text-red-600 mb-4">{auth.error}</p>
                <Button onclick={() => navigate({ type: 'login', state: {} })}>
                  Try Again
                </Button>
              </div>
            {:else if auth.isAuthenticated}
              <div>
                <p class="text-green-600 mb-4">Welcome, {auth.user?.email}!</p>
                <Button onclick={() => navigate({ type: 'dashboard', state: {} })}>
                  Go to Dashboard
                </Button>
              </div>
            {:else}
              <div>
                <p class="text-gray-600 mb-4">No token found in the URL. Please check your email for the magic link.</p>
                <Button variant="outline" onclick={() => navigate({ type: 'login', state: {} })}>
                  Request New Link
                </Button>
              </div>
            {/if}
          </CardContent>
        </Card>
      </div>

    {:else if destination.type === 'myReservations'}
      <!-- MY RESERVATIONS PAGE -->
      <div>
        <h1 class="text-3xl font-bold text-gray-900 mb-6">My Reservations</h1>

        {#if reservationsLoading}
          <div class="flex justify-center py-12">
            <Spinner size="lg" />
          </div>
        {:else if reservations.length === 0}
          <Card>
            <CardContent class="text-center py-12">
              <p class="text-gray-600 mb-4">You don't have any reservations yet.</p>
              <Button
                onclick={() => {
                  navigate({ type: 'events', state: {} });
                  store.dispatch({ type: 'events/loadList' });
                }}
              >
                Browse Events
              </Button>
            </CardContent>
          </Card>
        {:else}
          <div class="space-y-4">
            {#each reservations as reservation (reservation.id)}
              <Card>
                <CardContent class="py-4">
                  <div class="flex justify-between items-start">
                    <div>
                      <div class="font-medium">Reservation #{reservation.id.slice(0, 8)}</div>
                      <div class="text-sm text-gray-500">
                        {reservation.quantity} ticket(s) in {reservation.section} - {formatPrice(reservation.total_amount_cents)}
                      </div>
                      <div class="text-sm text-gray-500">
                        {new Date(reservation.created_at).toLocaleDateString()}
                      </div>
                    </div>
                    <Badge variant={
                      reservation.status === 'Confirmed' || reservation.status === 'confirmed' ? 'success' :
                      reservation.status === 'Pending' || reservation.status === 'pending' || reservation.status === 'PaymentPending' ? 'warning' :
                      'secondary'
                    }>
                      {reservation.status}
                    </Badge>
                  </div>
                  {#if reservation.status === 'Pending' || reservation.status === 'pending' || reservation.status === 'PaymentPending'}
                    <div class="mt-3 flex gap-2">
                      <Button
                        size="sm"
                        onclick={() => {
                          navigate({ type: 'reserve', state: { eventId: reservation.event_id } });
                        }}
                      >
                        Complete Payment
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        onclick={() => store.dispatch({ type: 'reservations/cancel', reservationId: reservation.id })}
                      >
                        Cancel
                      </Button>
                    </div>
                  {/if}
                </CardContent>
              </Card>
            {/each}
          </div>
        {/if}
      </div>

    {:else if destination.type === 'myPayments'}
      <!-- MY PAYMENTS PAGE -->
      <div>
        <h1 class="text-3xl font-bold text-gray-900 mb-6">Payment History</h1>

        {#if paymentsLoading}
          <div class="flex justify-center py-12">
            <Spinner size="lg" />
          </div>
        {:else if payments.length === 0}
          <Card>
            <CardContent class="text-center py-12">
              <p class="text-gray-600 mb-2">No payment history yet.</p>
              <p class="text-sm text-gray-500">Payments will appear here after you complete a purchase.</p>
            </CardContent>
          </Card>
        {:else}
          <div class="space-y-4">
            {#each payments as payment (payment.id)}
              <Card>
                <CardContent class="py-4">
                  <div class="flex justify-between items-start">
                    <div>
                      <div class="font-medium">{formatPrice(payment.amount_cents)}</div>
                      <div class="text-sm text-gray-500">
                        {payment.payment_method}
                      </div>
                      {#if payment.transaction_id}
                        <div class="text-xs text-gray-400">
                          Transaction: {payment.transaction_id}
                        </div>
                      {/if}
                    </div>
                    <Badge variant={
                      payment.status === 'succeeded' ? 'success' :
                      payment.status === 'processing' ? 'warning' :
                      payment.status === 'failed' ? 'destructive' :
                      'secondary'
                    }>
                      {payment.status}
                    </Badge>
                  </div>
                  {#if payment.failure_reason}
                    <p class="mt-2 text-sm text-red-600">{payment.failure_reason}</p>
                  {/if}
                </CardContent>
              </Card>
            {/each}
          </div>
        {/if}
      </div>

    {:else if destination.type === 'organizerEvents'}
      <!-- ORGANIZER EVENTS LIST PAGE -->
      <div>
        <div class="flex justify-between items-center mb-6">
          <h1 class="text-3xl font-bold text-gray-900">My Events</h1>
          <Button onclick={() => navigate({ type: 'createEvent', state: {} })}>
            Create Event
          </Button>
        </div>

        {#if organizer.myEventsLoading}
          <div class="space-y-4">
            {#each [1, 2, 3] as _}
              <Card>
                <CardContent class="py-4">
                  <Skeleton class="h-6 w-1/2 mb-2" />
                  <Skeleton class="h-4 w-1/3" />
                </CardContent>
              </Card>
            {/each}
          </div>
        {:else if organizer.myEventsError}
          <Card>
            <CardContent class="text-center py-8">
              <p class="text-red-600 mb-4">{organizer.myEventsError}</p>
              <Button variant="outline" onclick={() => store.dispatch({ type: 'organizer/loadMyEvents' })}>
                Try Again
              </Button>
            </CardContent>
          </Card>
        {:else if organizer.myEvents.length === 0}
          <Card>
            <CardContent class="text-center py-12">
              <p class="text-gray-600 mb-4">You haven't created any events yet.</p>
              <Button onclick={() => navigate({ type: 'createEvent', state: {} })}>
                Create Your First Event
              </Button>
            </CardContent>
          </Card>
        {:else}
          <div class="space-y-4">
            {#each organizer.myEvents as event}
              <Card>
                <CardContent class="py-4">
                  <div class="flex justify-between items-start">
                    <div class="flex-1">
                      <h3 class="text-lg font-semibold text-gray-900">{event.title}</h3>
                      <p class="text-sm text-gray-500">{event.venue.name}</p>
                      <p class="text-sm text-gray-500">{formatDate(event.start_time)} at {formatTime(event.start_time)}</p>
                    </div>
                    <div class="flex items-center gap-2">
                      <Badge variant={
                        event.status === 'published' ? 'success' :
                        event.status === 'draft' ? 'secondary' :
                        'destructive'
                      }>
                        {event.status}
                      </Badge>
                    </div>
                  </div>
                  <div class="flex gap-2 mt-4">
                    <Button
                      variant="outline"
                      size="sm"
                      onclick={() => navigate({ type: 'editEvent', state: { eventId: event.id } })}
                    >
                      Edit
                    </Button>
                    {#if event.status === 'draft'}
                      <Button
                        variant="default"
                        size="sm"
                        onclick={() => store.dispatch({ type: 'organizer/publishEvent', eventId: event.id })}
                      >
                        Publish
                      </Button>
                    {/if}
                    {#if event.status === 'published'}
                      <Button
                        variant="outline"
                        size="sm"
                        onclick={() => navigate({ type: 'eventAnalytics', state: { eventId: event.id } })}
                      >
                        Analytics
                      </Button>
                      <Button
                        variant="destructive"
                        size="sm"
                        onclick={() => store.dispatch({ type: 'organizer/cancelEvent', eventId: event.id })}
                      >
                        Cancel
                      </Button>
                    {/if}
                  </div>
                </CardContent>
              </Card>
            {/each}
          </div>
        {/if}
      </div>

    {:else if destination.type === 'createEvent'}
      <!-- CREATE EVENT PAGE -->
      <div class="max-w-2xl mx-auto">
        <div class="flex items-center gap-4 mb-6">
          <Button
            variant="ghost"
            size="sm"
            onclick={() => navigate({ type: 'organizerEvents', state: {} })}
          >
            &larr; Back
          </Button>
          <h1 class="text-3xl font-bold text-gray-900">Create Event</h1>
        </div>

        <Card>
          <CardContent class="py-6">
            <form
              onsubmit={(e: Event) => {
                e.preventDefault();
                store.dispatch({ type: 'organizer/createEvent' });
              }}
            >
              <div class="space-y-5">
                <div>
                  <label for="title" class="block text-sm font-medium text-gray-700 mb-1">
                    Event Title *
                  </label>
                  <Input
                    id="title"
                    type="text"
                    value={organizer.eventForm.title}
                    placeholder="Concert at Madison Square Garden"
                    required
                    disabled={organizer.formLoading}
                    oninput={(e: InputEvent) => {
                      const target = e.target as HTMLInputElement;
                      store.dispatch({ type: 'organizer/updateForm', field: 'title', value: target.value });
                    }}
                  />
                </div>

                <div>
                  <label for="description" class="block text-sm font-medium text-gray-700 mb-1">
                    Description
                  </label>
                  <textarea
                    id="description"
                    class="w-full min-h-[100px] px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:bg-gray-100 disabled:cursor-not-allowed"
                    value={organizer.eventForm.description}
                    placeholder="Tell attendees about your event..."
                    disabled={organizer.formLoading}
                    oninput={(e: InputEvent) => {
                      const target = e.target as HTMLTextAreaElement;
                      store.dispatch({ type: 'organizer/updateForm', field: 'description', value: target.value });
                    }}
                  ></textarea>
                </div>

                <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div>
                    <label for="startTime" class="block text-sm font-medium text-gray-700 mb-1">
                      Date & Time *
                    </label>
                    <Input
                      id="startTime"
                      type="datetime-local"
                      value={organizer.eventForm.startTime}
                      required
                      disabled={organizer.formLoading}
                      oninput={(e: InputEvent) => {
                        const target = e.target as HTMLInputElement;
                        store.dispatch({ type: 'organizer/updateForm', field: 'startTime', value: target.value });
                      }}
                    />
                  </div>

                  <div>
                    <label for="venueName" class="block text-sm font-medium text-gray-700 mb-1">
                      Venue *
                    </label>
                    <Input
                      id="venueName"
                      type="text"
                      value={organizer.eventForm.venueName}
                      placeholder="Madison Square Garden"
                      required
                      disabled={organizer.formLoading}
                      oninput={(e: InputEvent) => {
                        const target = e.target as HTMLInputElement;
                        store.dispatch({ type: 'organizer/updateForm', field: 'venueName', value: target.value });
                      }}
                    />
                  </div>
                </div>

                <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div>
                    <label for="capacity" class="block text-sm font-medium text-gray-700 mb-1">
                      Capacity *
                    </label>
                    <Input
                      id="capacity"
                      type="number"
                      min="1"
                      value={organizer.eventForm.capacity.toString()}
                      required
                      disabled={organizer.formLoading}
                      oninput={(e: InputEvent) => {
                        const target = e.target as HTMLInputElement;
                        store.dispatch({ type: 'organizer/updateForm', field: 'capacity', value: parseInt(target.value) || 0 });
                      }}
                    />
                  </div>

                  <div>
                    <label for="price" class="block text-sm font-medium text-gray-700 mb-1">
                      Ticket Price ($) *
                    </label>
                    <Input
                      id="price"
                      type="number"
                      min="0"
                      step="0.01"
                      value={organizer.eventForm.price.toString()}
                      required
                      disabled={organizer.formLoading}
                      oninput={(e: InputEvent) => {
                        const target = e.target as HTMLInputElement;
                        store.dispatch({ type: 'organizer/updateForm', field: 'price', value: parseFloat(target.value) || 0 });
                      }}
                    />
                  </div>
                </div>

                {#if organizer.formError}
                  <p class="text-red-600 text-sm">{organizer.formError}</p>
                {/if}

                <div class="flex gap-3 pt-4">
                  <Button
                    variant="outline"
                    type="button"
                    onclick={() => navigate({ type: 'organizerEvents', state: {} })}
                  >
                    Cancel
                  </Button>
                  <Button
                    type="submit"
                    class="flex-1"
                    disabled={organizer.formLoading || !organizer.eventForm.title || !organizer.eventForm.startTime || !organizer.eventForm.venueName}
                  >
                    {#if organizer.formLoading}
                      <Spinner size="sm" /> Creating...
                    {:else}
                      Create Event
                    {/if}
                  </Button>
                </div>
              </div>
            </form>
          </CardContent>
        </Card>
      </div>

    {:else if destination.type === 'editEvent'}
      <!-- EDIT EVENT PAGE -->
      <div class="max-w-2xl mx-auto">
        <div class="flex items-center gap-4 mb-6">
          <Button
            variant="ghost"
            size="sm"
            onclick={() => navigate({ type: 'organizerEvents', state: {} })}
          >
            &larr; Back
          </Button>
          <h1 class="text-3xl font-bold text-gray-900">Edit Event</h1>
        </div>

        {#if organizer.formLoading && !organizer.eventForm.title}
          <!-- Loading event data -->
          <Card>
            <CardContent class="py-12">
              <div class="flex justify-center">
                <Spinner size="lg" />
              </div>
              <p class="text-center text-gray-600 mt-4">Loading event details...</p>
            </CardContent>
          </Card>
        {:else}
          <Card>
            <CardContent class="py-6">
              <form
                onsubmit={(e: Event) => {
                  e.preventDefault();
                  store.dispatch({ type: 'organizer/updateEvent', eventId: destination.state.eventId });
                }}
              >
                <div class="space-y-5">
                  <div>
                    <label for="edit-title" class="block text-sm font-medium text-gray-700 mb-1">
                      Event Title *
                    </label>
                    <Input
                      id="edit-title"
                      type="text"
                      value={organizer.eventForm.title}
                      placeholder="Concert at Madison Square Garden"
                      required
                      disabled={organizer.formLoading}
                      oninput={(e: InputEvent) => {
                        const target = e.target as HTMLInputElement;
                        store.dispatch({ type: 'organizer/updateForm', field: 'title', value: target.value });
                      }}
                    />
                  </div>

                  <div>
                    <label for="edit-description" class="block text-sm font-medium text-gray-700 mb-1">
                      Description
                    </label>
                    <textarea
                      id="edit-description"
                      class="w-full min-h-[100px] px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-primary-500 disabled:bg-gray-100 disabled:cursor-not-allowed"
                      value={organizer.eventForm.description}
                      placeholder="Tell attendees about your event..."
                      disabled={organizer.formLoading}
                      oninput={(e: InputEvent) => {
                        const target = e.target as HTMLTextAreaElement;
                        store.dispatch({ type: 'organizer/updateForm', field: 'description', value: target.value });
                      }}
                    ></textarea>
                  </div>

                  <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div>
                      <label for="edit-startTime" class="block text-sm font-medium text-gray-700 mb-1">
                        Date & Time *
                      </label>
                      <Input
                        id="edit-startTime"
                        type="datetime-local"
                        value={organizer.eventForm.startTime}
                        required
                        disabled={organizer.formLoading}
                        oninput={(e: InputEvent) => {
                          const target = e.target as HTMLInputElement;
                          store.dispatch({ type: 'organizer/updateForm', field: 'startTime', value: target.value });
                        }}
                      />
                    </div>

                    <div>
                      <label for="edit-venueName" class="block text-sm font-medium text-gray-700 mb-1">
                        Venue *
                      </label>
                      <Input
                        id="edit-venueName"
                        type="text"
                        value={organizer.eventForm.venueName}
                        placeholder="Madison Square Garden"
                        required
                        disabled={organizer.formLoading}
                        oninput={(e: InputEvent) => {
                          const target = e.target as HTMLInputElement;
                          store.dispatch({ type: 'organizer/updateForm', field: 'venueName', value: target.value });
                        }}
                      />
                    </div>
                  </div>

                  <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div>
                      <label for="edit-capacity" class="block text-sm font-medium text-gray-700 mb-1">
                        Capacity *
                      </label>
                      <Input
                        id="edit-capacity"
                        type="number"
                        min="1"
                        value={organizer.eventForm.capacity.toString()}
                        required
                        disabled={organizer.formLoading}
                        oninput={(e: InputEvent) => {
                          const target = e.target as HTMLInputElement;
                          store.dispatch({ type: 'organizer/updateForm', field: 'capacity', value: parseInt(target.value) || 0 });
                        }}
                      />
                    </div>

                    <div>
                      <label for="edit-price" class="block text-sm font-medium text-gray-700 mb-1">
                        Ticket Price ($) *
                      </label>
                      <Input
                        id="edit-price"
                        type="number"
                        min="0"
                        step="0.01"
                        value={organizer.eventForm.price.toString()}
                        required
                        disabled={organizer.formLoading}
                        oninput={(e: InputEvent) => {
                          const target = e.target as HTMLInputElement;
                          store.dispatch({ type: 'organizer/updateForm', field: 'price', value: parseFloat(target.value) || 0 });
                        }}
                      />
                    </div>
                  </div>

                  {#if organizer.formError}
                    <p class="text-red-600 text-sm">{organizer.formError}</p>
                  {/if}

                  <div class="flex gap-3 pt-4">
                    <Button
                      variant="outline"
                      type="button"
                      onclick={() => navigate({ type: 'organizerEvents', state: {} })}
                    >
                      Cancel
                    </Button>
                    <Button
                      type="submit"
                      class="flex-1"
                      disabled={organizer.formLoading || !organizer.eventForm.title || !organizer.eventForm.startTime || !organizer.eventForm.venueName}
                    >
                      {#if organizer.formLoading}
                        <Spinner size="sm" /> Saving...
                      {:else}
                        Save Changes
                      {/if}
                    </Button>
                  </div>

                  <!-- Danger zone for delete -->
                  <div class="mt-8 pt-6 border-t border-gray-200">
                    <h3 class="text-sm font-medium text-gray-900 mb-2">Danger Zone</h3>
                    <p class="text-sm text-gray-500 mb-3">
                      Permanently delete this event. This action cannot be undone.
                    </p>
                    <Button
                      variant="destructive"
                      size="sm"
                      onclick={() => {
                        if (confirm('Are you sure you want to delete this event? This cannot be undone.')) {
                          store.dispatch({ type: 'organizer/deleteEvent', eventId: destination.state.eventId });
                        }
                      }}
                    >
                      Delete Event
                    </Button>
                  </div>
                </div>
              </form>
            </CardContent>
          </Card>
        {/if}
      </div>

    {:else if destination.type === 'eventAnalytics'}
      <!-- EVENT ANALYTICS PAGE -->
      <div>
        <h1 class="text-3xl font-bold text-gray-900 mb-6">Event Analytics</h1>

        <Card>
          <CardContent class="py-8">
            <p class="text-center text-gray-600 mb-4">
              Analytics for event: {destination.state.eventId}
            </p>
            <p class="text-center text-sm text-gray-500 mb-4">
              Analytics dashboard coming soon. You'll see ticket sales, revenue, and attendance data.
            </p>
            <div class="text-center">
              <Button
                variant="outline"
                onclick={() => navigate({ type: 'organizerEvents', state: {} })}
              >
                Back to My Events
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>

    {:else}
      <!-- 404 / NOT FOUND -->
      <Card>
        <CardContent class="text-center py-16">
          <h1 class="text-2xl font-bold text-gray-900 mb-4">Page Not Found</h1>
          <p class="text-gray-600 mb-8">The page you're looking for doesn't exist.</p>
          <Button onclick={() => navigate({ type: 'home', state: {} })}>
            Go Home
          </Button>
        </CardContent>
      </Card>
    {/if}
  </main>

  <!-- Footer -->
  <footer class="bg-white border-t mt-auto">
    <div class="mx-auto max-w-7xl px-4 py-8 sm:px-6 lg:px-8">
      <p class="text-center text-gray-500 text-sm">
        Built with Composable Svelte
      </p>
    </div>
  </footer>
</div>

<!-- Toast Container -->
{#if toasts.length > 0}
  <div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2 max-w-sm">
    {#each toasts as toast (toast.id)}
      <div
        class="flex items-start gap-3 p-4 rounded-lg shadow-lg border {
          toast.type === 'success' ? 'bg-green-50 border-green-200 text-green-800' :
          toast.type === 'error' ? 'bg-red-50 border-red-200 text-red-800' :
          toast.type === 'warning' ? 'bg-yellow-50 border-yellow-200 text-yellow-800' :
          'bg-blue-50 border-blue-200 text-blue-800'
        }"
        role="alert"
      >
        <!-- Icon -->
        <span class="flex-shrink-0 text-lg">
          {#if toast.type === 'success'}
            &#10003;
          {:else if toast.type === 'error'}
            &#10007;
          {:else if toast.type === 'warning'}
            &#9888;
          {:else}
            &#8505;
          {/if}
        </span>
        <!-- Message -->
        <p class="flex-1 text-sm">{toast.message}</p>
        <!-- Dismiss button -->
        <button
          type="button"
          class="flex-shrink-0 text-gray-400 hover:text-gray-600"
          onclick={() => store.dispatch({ type: 'ui/dismissToast', id: toast.id })}
        >
          &#10005;
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  @import 'tailwindcss/base';
  @import 'tailwindcss/components';
  @import 'tailwindcss/utilities';
</style>
