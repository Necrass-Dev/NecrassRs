use necrassrs::GraphQLInput;

#[allow(non_camel_case_types)]
pub struct Query;

#[allow(non_camel_case_types)]
pub struct Mutation;

#[allow(non_snake_case)]
impl crate::generated::resolvers::QueryResolver<crate::Context> for self::Query {}

#[allow(non_snake_case)]
impl crate::generated::resolvers::MutationResolver<crate::Context> for self::Mutation {}

impl necrassrs::Resolver<crate::generated::fields::Query::label, crate::Context> for self::Query {
    type Output = String;

    async fn resolve(
        &self,
        _context: &crate::Context,
        args: crate::generated::types::Query::label::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        match args.locator {
            crate::generated::types::Locator::id(id) => Ok(id.as_str().to_owned()),
            crate::generated::types::Locator::name(name) => Ok(name),
        }
    }
}

impl necrassrs::Resolver<crate::generated::fields::Query::statuses, crate::Context>
    for self::Query
{
    type Output = Vec<crate::generated::types::Status>;

    async fn resolve(
        &self,
        _context: &crate::Context,
        args: crate::generated::types::Query::statuses::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        let GraphQLInput::Value(filter) = args.filter else {
            return Ok(Vec::new());
        };
        match filter.status {
            GraphQLInput::Value(status) => Ok(vec![status]),
            GraphQLInput::Undefined | GraphQLInput::Null => Ok(Vec::new()),
        }
    }
}

impl necrassrs::Resolver<crate::generated::fields::Mutation::increment, crate::Context>
    for self::Mutation
{
    type Output = i32;

    async fn resolve(
        &self,
        context: &crate::Context,
        args: crate::generated::types::Mutation::increment::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        let mut counter = context
            .counter
            .lock()
            .map_err(|_| necrassrs::ResolverError::new("Counter is unavailable"))?;
        *counter = counter
            .checked_add(args.by)
            .ok_or_else(|| necrassrs::ResolverError::new("Counter overflow"))?;
        Ok(*counter)
    }
}
