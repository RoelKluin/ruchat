use crate::error::{Result, RuChatError};
use std::collections::HashMap;

/// Represents an answer in the conversation tree.
#[derive(Debug, Clone)]
struct Answer {
    id: usize,
    text: Vec<String>,
    response_counter: usize,
    parent_question_id: usize,
    next_edited_answer_id: Option<usize>,
    prev_edited_answer_id: Option<usize>,
}

/// Represents a question in the conversation tree.
#[derive(Debug, Clone)]
struct Question {
    id: usize,
    text: Vec<String>,
    answers: Vec<usize>, // Store answer IDs
    current_answer_id: usize,
    parent_question_id: Option<usize>,
    children_question_ids: Vec<usize>,
    next_edited_question_id: Option<usize>,
    prev_edited_question_id: Option<usize>,
}

/// A struct for managing a conversation tree structure.
///
/// This struct provides methods for adding and editing questions and answers,
/// navigating the conversation tree, and retrieving question and answer details.
#[derive(Debug, Clone)]
pub(crate) struct ConversationTree {
    questions: HashMap<usize, Question>,
    answers: HashMap<usize, Answer>,
    current_question_ids: Vec<usize>,
    at_question: usize,
    next_question_id: usize,
    next_answer_id: usize,
}

impl ConversationTree {
    /// Creates a new `ConversationTree` instance.
    ///
    /// # Returns
    ///
    /// A new instance of `ConversationTree` with empty questions and answers.
    pub(crate) fn new() -> Self {
        ConversationTree {
            questions: HashMap::new(),
            answers: HashMap::new(),
            current_question_ids: vec![],
            at_question: 0, // one-based index, 0 is not a valid question
            next_question_id: 0,
            next_answer_id: 0,
        }
    }

    /// Adds a new question to the conversation tree.
    ///
    /// This function creates a new question and a default answer, and adds
    /// them to the conversation tree.
    ///
    /// # Parameters
    ///
    /// - `question_text`: The text of the question to add.
    ///
    /// # Returns
    ///
    /// A `Result` containing the question ID or a `RuChatError`.
    pub(crate) fn question(&mut self, question_text: Vec<String>) -> Result<usize> {
        let question_id = self.next_question_id;
        self.next_question_id += 1;
        let answer_id = self.next_answer_id;

        let question = Question {
            id: question_id,
            text: question_text,
            answers: vec![answer_id],
            current_answer_id: answer_id,
            parent_question_id: None,
            children_question_ids: Vec::new(),
            next_edited_question_id: None,
            prev_edited_question_id: None,
        };

        self.questions
            .insert(question_id, question)
            .map_or(Ok(()), |_| Err(RuChatError::QuestionAlreadyExists))?;

        // Create a default answer
        let answer = Answer {
            id: answer_id,
            text: Vec::new(),
            response_counter: 0,
            parent_question_id: question_id,
            next_edited_answer_id: None,
            prev_edited_answer_id: None,
        };

        self.answers
            .insert(answer_id, answer)
            .map_or(Ok(()), |_| Err(RuChatError::AnswerAlreadyExists))?;

        self.current_question_ids.push(question_id);
        self.at_question += 1;
        Ok(question_id)
    }

    /// Edits an existing question in the conversation tree.
    ///
    /// This function creates a new version of the question with the specified
    /// text and updates the conversation tree.
    ///
    /// # Parameters
    ///
    /// - `question_id`: The ID of the question to edit.
    /// - `new_question_text`: The new text for the question.
    ///
    /// # Returns
    ///
    /// A `Result` containing the new question ID or a `RuChatError`.
    pub(crate) fn edit_question(
        &mut self,
        question_id: usize,
        new_question_text: Vec<String>,
    ) -> Result<usize> {
        let question = self
            .questions
            .get_mut(&question_id)
            .ok_or(RuChatError::QuestionNotFound)?;

        let new_question_id = self.next_question_id;
        let new_answer_id = self.next_answer_id;
        question.next_edited_question_id = Some(new_question_id);

        let new_question = Question {
            id: new_question_id,
            text: new_question_text,
            answers: vec![new_answer_id],
            current_answer_id: new_answer_id,
            parent_question_id: Some(question_id),
            children_question_ids: Vec::new(),
            next_edited_question_id: None,
            prev_edited_question_id: Some(question_id),
        };

        self.questions
            .insert(new_question_id, new_question)
            .map_or(Ok(()), |_| Err(RuChatError::QuestionAlreadyExists))?;

        let new_answer = Answer {
            id: new_answer_id,
            text: Vec::new(),
            response_counter: 0,
            parent_question_id: new_question_id,
            next_edited_answer_id: None,
            prev_edited_answer_id: None,
        };

        self.answers
            .insert(new_answer_id, new_answer)
            .map_or(Ok(()), |_| Err(RuChatError::AnswerAlreadyExists))?;

        if let Some(parent_qid) = self.find_parent(question_id) {
            let parent_question = self
                .questions
                .get_mut(&parent_qid)
                .ok_or(RuChatError::QuestionNotFound)?;
            parent_question.children_question_ids.push(new_question_id);
        }
        *self
            .current_question_ids
            .get_mut(self.at_question - 1)
            .ok_or(RuChatError::QuestionNotFound)? = new_question_id;
        self.next_question_id += 1;
        Ok(new_question_id)
    }

    /// Retrieves the current question IDs in the conversation tree.
    ///
    /// # Returns
    ///
    /// A reference to a vector of current question IDs.
    pub(crate) fn get_current_question_ids(&self) -> &Vec<usize> {
        &self.current_question_ids
    }

    /// Adds an answer to a question in the conversation tree.
    ///
    /// This function adds a new answer to the specified question and updates
    /// the conversation tree.
    ///
    /// # Parameters
    ///
    /// - `question_id`: The ID of the question to add the answer to.
    /// - `text`: The text of the answer to add.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) fn add_answer(&mut self, question_id: usize, text: Vec<String>) -> Result<()> {
        let question = self
            .questions
            .get_mut(&question_id)
            .ok_or(RuChatError::QuestionNotFound)?;
        // prevents hot loop in get_answer_nr_of_total when using add_answer() in place of answer()
        if self
            .answers
            .get_mut(&question.current_answer_id)
            .filter(|old_answer| old_answer.text.is_empty())
            .is_some()
        {
            let answer_id = question.current_answer_id;
            let answer = self
                .answers
                .get_mut(&answer_id)
                .ok_or(RuChatError::AnswerNotFound)?;
            answer.text = text;
            answer.response_counter += 1;
        } else {
            let new_answer_id = self.next_answer_id;

            let new_answer = Answer {
                id: new_answer_id,
                text,
                response_counter: question.answers.len() + 1,
                parent_question_id: question_id,
                next_edited_answer_id: None,
                prev_edited_answer_id: Some(question.current_answer_id),
            };

            question.answers.push(new_answer_id);
            question.current_answer_id = new_answer_id;
            self.answers.insert(new_answer_id, new_answer);
        }
        self.next_answer_id += 1;
        Ok(())
    }

    /// Retrieves the question number and total count for a question.
    ///
    /// This function returns a string representing the question number
    /// and total count in the format "[current/total]".
    ///
    /// # Parameters
    ///
    /// - `question_id`: The ID of the question to retrieve the number for.
    ///
    /// # Returns
    ///
    /// A `String` representing the question number and total count.
    pub(crate) fn get_question_nr_of_total(&self, question_id: usize) -> String {
        if let Some(question) = self.questions.get(&question_id) {
            let mut prev_next = question.prev_edited_question_id;
            let mut count = 1;
            while let Some(q) = prev_next.and_then(|prev| self.questions.get(&prev)) {
                count += 1;
                prev_next = q.prev_edited_question_id;
            }

            let mut total = count;
            prev_next = question.next_edited_question_id;
            while let Some(q) = prev_next.and_then(|prev| self.questions.get(&prev)) {
                total += 1;
                prev_next = q.next_edited_question_id;
            }
            let mut ret = String::from("[");
            if count > 1 {
                ret.push('▼');
            }
            ret.push_str(count.to_string().as_str());
            if total > count {
                ret.push('▲');
            }
            ret.push('/');
            ret.push_str(total.to_string().as_str());
            ret.push(']');
            ret
        } else {
            String::from("[0/0]")
        }
    }

    /// Retrieves the answer number and total count for an answer.
    ///
    /// This function returns a string representing the answer number
    /// and total count in the format "[current/total]".
    ///
    /// # Parameters
    ///
    /// - `answer_id`: The ID of the answer to retrieve the number for.
    ///
    /// # Returns
    ///
    /// A `String` representing the answer number and total count.
    pub(crate) fn get_answer_nr_of_total(&self, answer_id: usize) -> String {
        if let Some(answer) = self.answers.get(&answer_id) {
            let mut prev_next = answer.prev_edited_answer_id;
            let mut count = 1;
            while let Some(q) = prev_next.and_then(|prev| self.answers.get(&prev)) {
                count += 1;
                prev_next = q.prev_edited_answer_id;
            }
            let mut total = count;
            prev_next = answer.next_edited_answer_id;
            while let Some(q) = prev_next.and_then(|prev| self.answers.get(&prev)) {
                total += 1;
                prev_next = q.next_edited_answer_id;
            }
            let mut ret = String::from("[");
            if count > 1 {
                ret.push('▼');
            }
            ret.push_str(count.to_string().as_str());
            if total > count {
                ret.push('▲');
            }
            ret.push('/');
            ret.push_str(total.to_string().as_str());
            ret.push(']');
            ret
        } else {
            String::from("[0/0]")
        }
    }

    /// Finds the parent question ID for a given question.
    ///
    /// # Parameters
    ///
    /// - `question_id`: The ID of the question to find the parent for.
    ///
    /// # Returns
    ///
    /// An `Option` containing the parent question ID, or `None` if not found.
    pub(crate) fn find_parent(&self, question_id: usize) -> Option<usize> {
        self.questions
            .get(&question_id)
            .and_then(|q| q.parent_question_id)
    }

    /// Retrieves the question and answer text for a given question and answer ID.
    ///
    /// # Parameters
    ///
    /// - `question_id`: The ID of the question to retrieve the text for.
    /// - `answer_id`: The ID of the answer to retrieve the text for.
    ///
    /// # Returns
    ///
    /// An `Option` containing a tuple of question and answer text, or `None` if not found.
    pub(crate) fn get_qa(
        &self,
        question_id: usize,
        answer_id: usize,
    ) -> Option<(Vec<String>, Vec<String>)> {
        if let Some(question) = self.questions.get(&question_id) {
            let question_text = question.text.clone();
            let response_text = self
                .answers
                .get(&answer_id)
                .map_or(Vec::new(), |answer| answer.text.clone());
            Some((question_text, response_text))
        } else {
            None
        }
    }

    /// Retrieves the current question ID in the conversation tree.
    ///
    /// # Returns
    ///
    /// A `Result` containing the current question ID or a `RuChatError`.
    pub(crate) fn get_current_question_id(&self) -> Result<usize> {
        self.current_question_ids
            .get(self.at_question - 1)
            .ok_or(RuChatError::QuestionNotFound)
            .copied()
    }

    /// Retrieves the current answer ID for a given question.
    ///
    /// # Parameters
    ///
    /// - `current_question_id`: The ID of the current question.
    ///
    /// # Returns
    ///
    /// The current answer ID for the specified question.
    pub(crate) fn get_current_answer_id(&self, current_question_id: usize) -> usize {
        self.questions
            .get(&current_question_id)
            .map_or(0, |question| question.current_answer_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_question() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        assert_eq!(tree.questions.len(), 1);
        assert_eq!(tree.questions[&question_id].text, question_text);
    }

    #[test]
    fn test_edit_question() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let new_question_text = vec!["What is your full name?".to_string()];
        let new_question_id = tree
            .edit_question(question_id, new_question_text.clone())
            .unwrap();
        assert_eq!(tree.questions.len(), 2);
        assert_eq!(tree.questions[&new_question_id].text, new_question_text);
    }

    #[test]
    fn test_add_answer() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        tree.add_answer(question_id, answer_text.clone()).unwrap();
        assert_eq!(tree.answers.len(), 1);
        assert_eq!(tree.answers[&0].text, answer_text);
    }
    #[test]
    fn test_get_question_nr_of_total() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        tree.add_answer(question_id, answer_text.clone()).unwrap();
        assert_eq!(tree.get_question_nr_of_total(question_id), "[1/1]");
    }

    #[test]
    fn test_get_answer_nr_of_total() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        tree.add_answer(question_id, answer_text.clone()).unwrap();
        assert_eq!(tree.get_answer_nr_of_total(0), "[1/1]");
    }

    #[test]
    fn test_find_parent() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        tree.add_answer(question_id, answer_text.clone()).unwrap();
        assert_eq!(tree.find_parent(question_id), None);
    }

    #[test]
    fn test_get_qa() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        tree.add_answer(question_id, answer_text.clone()).unwrap();
        let qa = tree.get_qa(question_id, 0).unwrap();
        assert_eq!(qa.0, question_text);
        assert_eq!(qa.1, answer_text);
    }

    #[test]
    fn test_get_current_question_id() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        assert_eq!(tree.get_current_question_id().unwrap(), question_id);
    }

    #[test]
    fn test_get_current_answer_id() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        tree.add_answer(question_id, answer_text.clone()).unwrap();
        assert_eq!(tree.get_current_answer_id(question_id), 0);
    }

    #[test]
    fn test_add_answer_to_nonexistent_question() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        assert!(tree.add_answer(question_id + 1, answer_text).is_err());
    }

    #[test]
    fn test_edit_question_not_found() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let new_question_text = vec!["What is your full name?".to_string()];
        assert!(
            tree.edit_question(question_id + 1, new_question_text)
                .is_err()
        );
    }

    #[test]
    fn test_add_answer_not_found() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        assert!(tree.add_answer(question_id + 1, answer_text).is_err());
    }

    #[test]
    fn test_get_qa_not_found() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        tree.add_answer(question_id, answer_text.clone()).unwrap();
        assert!(tree.get_qa(question_id + 1, 0).is_none());
    }

    #[test]
    fn test_get_current_question_id_not_found() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        assert!(tree.get_current_question_id().is_ok());
        tree.current_question_ids.push(question_id + 1);
        assert!(tree.get_current_question_id().is_err());
    }

    #[test]
    fn test_get_current_answer_id_not_found() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        let answer_text = vec!["John Doe".to_string()];
        tree.add_answer(question_id, answer_text.clone()).unwrap();
        assert_eq!(tree.get_current_answer_id(question_id), 0);
        assert_eq!(tree.get_current_answer_id(question_id + 1), 0);
    }

    #[test]
    fn test_find_parent_not_found() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        assert_eq!(tree.find_parent(question_id + 1), None);
    }

    #[test]
    fn test_get_question_nr_of_total_not_found() {
        let mut tree = ConversationTree::new();
        let question_text = vec!["What is your name?".to_string()];
        let question_id = tree.question(question_text.clone()).unwrap();
        assert_eq!(tree.get_question_nr_of_total(question_id + 1), "[0/0]");
    }
}
