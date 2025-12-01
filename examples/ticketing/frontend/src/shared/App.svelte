<script lang="ts">
  import type { Store } from '@composable-svelte/core';
  import type { AppState, Event, EventSummary } from './types';
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
    Progress
  } from '@composable-svelte/core/components/ui';

  // Navigation components can be imported from '@composable-svelte/core/navigation-components' when needed
  // Available: Modal, Sheet, Alert, Drawer, Popover, Sidebar, Tabs, NavigationStack

  // Props
  interface Props {
    store: Store<AppState, AppAction>;
  }

  let { store }: Props = $props();

  // Reactive state from store using $store auto-subscription
  const destination = $derived($store.destination);
  const auth = $derived($store.auth);
  const meta = $derived($store.meta);
  const events = $derived($store.events);
  const eventsLoading = $derived($store.eventsLoading);
  const eventsError = $derived($store.eventsError);
  const selectedEvent = $derived($store.selectedEvent);
  const availability = $derived($store.availability);
  const reservations = $derived($store.reservations);
  const currentReservation = $derived($store.currentReservation);
  const reservationsLoading = $derived($store.reservationsLoading);
  const reservationsError = $derived($store.reservationsError);
  const payments = $derived($store.payments);
  const paymentsLoading = $derived($store.paymentsLoading);
  const paymentsError = $derived($store.paymentsError);

  // Auto-load data when navigating to certain pages
  $effect(() => {
    // Load events list when navigating to events page (including direct URL access)
    if (destination.type === 'events' && events.length === 0 && !eventsLoading && !eventsError) {
      store.dispatch({ type: 'events/loadList' });
    }
    // Load event detail when navigating to event detail page
    if (destination.type === 'eventDetail' && destination.state.eventId && !selectedEvent && !eventsLoading) {
      store.dispatch({ type: 'events/loadDetail', eventId: destination.state.eventId });
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
            <a
              href={urls.dashboard()}
              class="text-gray-600 hover:text-gray-900"
              onclick={(e: MouseEvent) => {
                e.preventDefault();
                navigate({ type: 'dashboard', state: {} });
              }}
            >
              Dashboard
            </a>
            <Button
              variant="ghost"
              onclick={() => store.dispatch({ type: 'auth/logout' })}
            >
              Logout
            </Button>
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
                  <CardTitle>{event.name}</CardTitle>
                  <CardDescription>
                    {event.venue_name} &middot; {formatDate(event.date)}
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div class="flex justify-between items-center">
                    <Badge variant={event.status === 'Published' ? 'success' : 'secondary'}>
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
                          <span>{section.available}/{section.total_capacity}</span>
                        </div>
                        <Progress value={(section.available / section.total_capacity) * 100} />
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
                <div class="text-green-600 mb-2">Check your email!</div>
                <p class="text-gray-600 text-sm">
                  We've sent you a magic link. Click it to sign in.
                </p>
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
          <Card>
            <CardContent class="py-8 text-center text-gray-600">
              <p>Your payment history will appear here.</p>
            </CardContent>
          </Card>
        {:else}
          <Card>
            <CardContent class="py-8 text-center text-gray-600">
              <p>Your reservations will appear here.</p>
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
            <CardTitle>Verifying...</CardTitle>
            <CardDescription>Please wait while we verify your login</CardDescription>
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
                <p class="text-green-600 mb-4">Successfully logged in!</p>
                <Button onclick={() => navigate({ type: 'dashboard', state: {} })}>
                  Go to Dashboard
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
                        {reservation.seats.length} ticket(s) - {formatPrice(reservation.total_amount_cents)}
                      </div>
                      <div class="text-sm text-gray-500">
                        {new Date(reservation.created_at).toLocaleDateString()}
                      </div>
                    </div>
                    <Badge variant={
                      reservation.status === 'confirmed' ? 'success' :
                      reservation.status === 'pending' || reservation.status === 'payment_pending' ? 'warning' :
                      'secondary'
                    }>
                      {reservation.status}
                    </Badge>
                  </div>
                  {#if reservation.status === 'pending' || reservation.status === 'payment_pending'}
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
              <p class="text-gray-600">No payment history yet.</p>
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
                        {payment.payment_method.type}
                        {#if payment.payment_method.last_four}
                          ending in {payment.payment_method.last_four}
                        {/if}
                      </div>
                      <div class="text-sm text-gray-500">
                        {new Date(payment.created_at).toLocaleDateString()}
                      </div>
                    </div>
                    <Badge variant={
                      payment.status === 'succeeded' ? 'success' :
                      payment.status === 'pending' || payment.status === 'processing' ? 'warning' :
                      'secondary'
                    }>
                      {payment.status}
                    </Badge>
                  </div>
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

        <Card>
          <CardContent class="text-center py-12">
            <p class="text-gray-600 mb-4">Event organizer features coming soon.</p>
            <p class="text-sm text-gray-500">
              You'll be able to create and manage events, view analytics, and more.
            </p>
          </CardContent>
        </Card>
      </div>

    {:else if destination.type === 'createEvent'}
      <!-- CREATE EVENT PAGE -->
      <div class="max-w-2xl mx-auto">
        <h1 class="text-3xl font-bold text-gray-900 mb-6">Create Event</h1>

        <Card>
          <CardContent class="py-8">
            <p class="text-center text-gray-600 mb-4">Event creation form coming soon.</p>
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

    {:else if destination.type === 'editEvent'}
      <!-- EDIT EVENT PAGE -->
      <div class="max-w-2xl mx-auto">
        <h1 class="text-3xl font-bold text-gray-900 mb-6">Edit Event</h1>

        <Card>
          <CardContent class="py-8">
            <p class="text-center text-gray-600 mb-4">
              Editing event: {destination.state.eventId}
            </p>
            <p class="text-center text-sm text-gray-500 mb-4">Event editing form coming soon.</p>
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

<style>
  @import 'tailwindcss/base';
  @import 'tailwindcss/components';
  @import 'tailwindcss/utilities';
</style>
