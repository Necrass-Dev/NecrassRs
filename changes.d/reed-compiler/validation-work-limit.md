 -  Fixed excessive executable-document validation work when multiple operations
    reuse shared fragments. Documents that exceed the validation work limit now
    return a validation error; applications may need to reduce repeated work in
    such documents.
