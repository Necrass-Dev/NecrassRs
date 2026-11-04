use futures::{StreamExt, stream};
use necrassrs::{GraphQLInput, Id, ResolverError, SourceEvent, SourceStream};
use std::sync::atomic::{AtomicUsize, Ordering};

mod scalars;
pub use self::scalars::Token;

pub struct AppContext {
    pub label: String,
    pub creates: AtomicUsize,
}
pub struct RootQuery;
pub struct RootMutation;
pub struct Feed;
pub struct Order {
    pub id: Id,
    pub status: crate::generated::types::Status,
}
pub struct Notice {
    pub text: String,
}

impl crate::generated::resolvers::RootQueryResolver<AppContext> for RootQuery {}
impl crate::generated::resolvers::RootMutationResolver<AppContext> for RootMutation {}
#[allow(non_snake_case)]
impl crate::generated::resolvers::FeedResolver<AppContext> for Feed {}

impl necrassrs::Resolver<crate::generated::fields::RootQuery::ping, AppContext> for RootQuery {
    type Output = i32;
    async fn resolve(
        &self,
        _: &AppContext,
        _: crate::generated::types::RootQuery::ping::Args,
    ) -> Result<Self::Output, ResolverError> {
        Ok(1)
    }
}

impl necrassrs::Resolver<crate::generated::fields::RootMutation::update, AppContext>
    for RootMutation
{
    type Output = i32;
    async fn resolve(
        &self,
        _: &AppContext,
        _: crate::generated::types::RootMutation::update::Args,
    ) -> Result<Self::Output, ResolverError> {
        Ok(2)
    }
}

impl necrassrs::Subscriber<crate::generated::fields::Feed::orderChanged, AppContext> for Feed {
    type Output = Order;
    async fn resolve<'a>(
        &'a self,
        context: &'a AppContext,
        args: crate::generated::types::Feed::orderChanged::Args,
    ) -> Result<SourceStream<'a, Self::Output>, ResolverError> {
        context.creates.fetch_add(1, Ordering::SeqCst);
        assert_eq!(args.token.0, "allowed");
        assert!(matches!(
            args.filter.note,
            GraphQLInput::Undefined | GraphQLInput::Null
        ));
        let id = args.filter.id;
        let events = args
            .filter
            .states
            .into_iter()
            .map(move |status| {
                SourceEvent::Value(Order {
                    id: id.clone(),
                    status,
                })
            })
            .collect::<Vec<_>>();
        Ok(SourceStream::from_stream(stream::iter(events).map(
            move |event| {
                assert!(!context.label.is_empty());
                event
            },
        )))
    }
}

impl necrassrs::Subscriber<crate::generated::fields::Feed::updates, AppContext> for Feed {
    type Output = crate::generated::types::Update<Order, Notice>;
    async fn resolve<'a>(
        &'a self,
        context: &'a AppContext,
        _: crate::generated::types::Feed::updates::Args,
    ) -> Result<SourceStream<'a, Self::Output>, ResolverError> {
        context.creates.fetch_add(1, Ordering::SeqCst);
        Ok(SourceStream::from_stream(stream::iter([
            SourceEvent::Value(crate::generated::types::Update::Order(Order {
                id: Id::from("42"),
                status: crate::generated::types::Status::PAID,
            })),
            SourceEvent::Value(crate::generated::types::Update::Notice(Notice {
                text: "Delivered".into(),
            })),
        ])))
    }
}

impl necrassrs::Subscriber<crate::generated::fields::Feed::count, AppContext> for Feed {
    type Output = i32;
    async fn resolve<'a>(
        &'a self,
        context: &'a AppContext,
        _: crate::generated::types::Feed::count::Args,
    ) -> Result<SourceStream<'a, Self::Output>, ResolverError> {
        context.creates.fetch_add(1, Ordering::SeqCst);
        Ok(SourceStream::from_stream(stream::iter([
            SourceEvent::Value(1),
            SourceEvent::Value(2),
        ])))
    }
}

impl necrassrs::Resolver<crate::generated::fields::Order::id, AppContext> for Order {
    type Output = Id;
    async fn resolve(
        &self,
        _: &AppContext,
        _: crate::generated::types::Order::id::Args,
    ) -> Result<Self::Output, ResolverError> {
        Ok(self.id.clone())
    }
}
impl necrassrs::Resolver<crate::generated::fields::Order::status, AppContext> for Order {
    type Output = crate::generated::types::Status;
    async fn resolve(
        &self,
        _: &AppContext,
        _: crate::generated::types::Order::status::Args,
    ) -> Result<Self::Output, ResolverError> {
        Ok(self.status.clone())
    }
}
impl necrassrs::Resolver<crate::generated::fields::Order::label, AppContext> for Order {
    type Output = String;
    async fn resolve(
        &self,
        context: &AppContext,
        _: crate::generated::types::Order::label::Args,
    ) -> Result<Self::Output, ResolverError> {
        Ok(context.label.clone())
    }
}
impl necrassrs::Resolver<crate::generated::fields::Notice::text, AppContext> for Notice {
    type Output = String;
    async fn resolve(
        &self,
        _: &AppContext,
        _: crate::generated::types::Notice::text::Args,
    ) -> Result<Self::Output, ResolverError> {
        Ok(self.text.clone())
    }
}
